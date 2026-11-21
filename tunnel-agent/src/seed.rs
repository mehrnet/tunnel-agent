use std::{fs, io::Write, net::TcpStream, path::Path, process::Command};

use crate::manifest::{Config, NodeScope};
use crate::seed_client::SeedSource;
use crate::{
    agent_relays,
    identity::valid_uuid,
    release::{
        BundleFile, BundleIndex, INDEX_NAME, ReleaseDocument, as_utf8, cache_link_or_copy,
        ensure_index_matches_document, local_architecture, parse_index, set_mode, sha256_bytes,
        unique_suffix, verify_file, verify_release_document,
    },
};

const MAX_DOCUMENT_BYTES: u64 = 16_384;
pub(crate) const MAX_RELAY_DOCUMENT_BYTES: u64 = 512 * 1024;
const MAX_INDEX_BYTES: u64 = 1_048_576;
pub(crate) use crate::seed_server::serve;

pub(crate) struct SeedResult {
    pub(crate) version: String,
    pub(crate) downloaded_files: usize,
    pub(crate) reused_files: usize,
    pub(crate) changed: bool,
    pub(crate) activated: bool,
}

pub(crate) fn ping_relay(source: &str, node_id: &str) -> Result<(), String> {
    if !valid_uuid(node_id) {
        return Err("relay node id is invalid".to_owned());
    }
    SeedSource::with_timeout(source, crate::agent_poll::CONFIG_REQUEST_TIMEOUT)?
        .get_bytes(&format!("/v1/ping/{node_id}"), 0)?;
    Ok(())
}

pub(crate) fn apply(root: &Path, source: &str, activate: bool) -> Result<SeedResult, String> {
    let document = release_from(source)?;
    apply_document(root, source, &document, activate)
}

fn release_from(source: &str) -> Result<ReleaseDocument, String> {
    let architecture = local_architecture()?;
    let raw = SeedSource::with_timeout(source, crate::agent_poll::CONFIG_REQUEST_TIMEOUT)?
        .get_bytes("/v1/release", MAX_DOCUMENT_BYTES)?;
    verify_release_document(as_utf8(&raw, "release document")?, &architecture)
}

fn apply_document(
    root: &Path,
    source: &str,
    document: &ReleaseDocument,
    activate: bool,
) -> Result<SeedResult, String> {
    let source = SeedSource::new(source)?;
    let raw_index = source.get_bytes("/v1/index", MAX_INDEX_BYTES)?;
    if sha256_bytes(&raw_index) != document.inventory_sha256 {
        return Err("seed inventory does not match the signed release document".to_owned());
    }
    let index = parse_index(as_utf8(&raw_index, "bundle inventory")?)?;
    ensure_index_matches_document(&index, document)?;
    install_seeded_release(root, &source, document, &index, &raw_index, activate)
}

pub(crate) struct StagedSeed {
    source: String,
    version: String,
}

pub(crate) fn stage_configured(config: &Config) -> Result<Option<StagedSeed>, String> {
    if config.node_scope != NodeScope::Domestic {
        return Err("only domestic agents may apply a peer-seeded bundle".to_owned());
    }
    let sources = agent_relays::sources(&config.agent);
    if sources.is_empty() {
        return Ok(None);
    }
    let root = configured_root(config)?;
    let active = fs::canonicalize(root.join("current"))
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        });
    let results = agent_relays::collect(&sources, |source| {
        release_from(source).map(|document| (source.to_owned(), document))
    });
    let mut candidates = Vec::new();
    let mut error = "no relay can serve a signed bundle".to_owned();
    for result in results {
        match result {
            Ok(candidate) => candidates.push(candidate),
            Err(failure) => error = failure,
        }
    }
    if candidates.is_empty() {
        return Err(error);
    }
    let redundant = sources.len() > 1;
    candidates.sort_by_key(|(_, doc)| std::cmp::Reverse(release_number(&doc.version)));
    let mut attempted = false;
    for (source, document) in candidates {
        if !can_stage(&document.version, active.as_deref(), redundant) {
            continue;
        }
        attempted = true;
        match apply_document(root, &source, &document, false) {
            Ok(result) => {
                return Ok(Some(StagedSeed {
                    source,
                    version: result.version,
                }));
            }
            Err(failure) => error = format!("{source}: {failure}"),
        }
    }
    if attempted { Err(error) } else { Ok(None) }
}

fn release_number(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.strip_prefix('v')?.split('.');
    let number = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(number)
}

fn can_stage(version: &str, active: Option<&str>, redundant: bool) -> bool {
    if active == Some(version) {
        return false;
    }
    // Two valid signatures say nothing about cache age. Only accept an upgrade on redundant lanes.
    !redundant
        || active.is_none_or(|active| {
            release_number(version)
                .is_some_and(|next| release_number(active).is_none_or(|old| next > old))
        })
}

#[test]
fn stale_signed_release_cannot_downgrade_a_redundant_agent() {
    assert!(can_stage("v0.6.0", Some("v0.5.9"), true));
    assert!(can_stage("v0.5.10", Some("v0.5.9"), true));
    assert!(!can_stage("v0.5.9", Some("v0.6.0"), true));
    assert!(!can_stage("v0.6.0", Some("v0.6.0"), true));
    assert!(can_stage("v0.5.9", Some("v0.6.0"), false));
}

pub(crate) fn activate_configured(
    config: &Config,
    staged: Result<Option<StagedSeed>, String>,
) -> Result<Option<String>, String> {
    let Some(StagedSeed { source, version }) = staged? else {
        return Ok(None);
    };
    if !agent_relays::sources(&config.agent).contains(&source) {
        return Ok(None);
    }
    let root = configured_root(config)?;
    activate_release(root, &root.join("releases").join(&version))?;
    Ok(Some(version))
}

fn configured_root(config: &Config) -> Result<&Path, String> {
    config
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.root.parent())
        .ok_or_else(|| "bundle.root has no agent root directory".to_owned())
}

pub(crate) fn restart_service(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@._-".contains(&byte))
    {
        return Err("service name contains unsafe characters".to_owned());
    }
    let status = Command::new("systemctl")
        .arg("restart")
        .arg(name)
        .status()
        .map_err(|error| format!("cannot invoke systemctl: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl restart {name} exited with {status}"))
    }
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    write!(stream, "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n", body.len())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| format!("cannot write seed response: {error}"))
}

fn install_seeded_release(
    root: &Path,
    source: &SeedSource,
    document: &ReleaseDocument,
    index: &BundleIndex,
    raw_index: &[u8],
    activate: bool,
) -> Result<SeedResult, String> {
    let release_dir = root.join("releases").join(&document.version);
    if release_dir.exists() {
        verify_installed_release(&release_dir, index, raw_index, document)?;
        let changed = activate && !is_current_release(root, &release_dir);
        if activate {
            activate_release(root, &release_dir)?;
        }
        return Ok(SeedResult {
            version: document.version.clone(),
            downloaded_files: 0,
            reused_files: index.files.len(),
            changed,
            activated: activate,
        });
    }
    let cache = root.join("cache").join("blobs");
    fs::create_dir_all(&cache).map_err(|error| format!("cannot create blob cache: {error}"))?;
    prime_cache_from_current(root, &cache)?;
    let stage = root
        .join("releases")
        .join(format!(".{}-{}", document.version, unique_suffix()?));
    fs::create_dir_all(&stage).map_err(|error| format!("cannot create release stage: {error}"))?;
    let result = materialize_stage(&stage, &cache, source, index, raw_index);
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    let (downloaded_files, reused_files) = result?;
    fs::rename(&stage, &release_dir)
        .map_err(|error| format!("cannot publish seeded release: {error}"))?;
    if activate {
        activate_release(root, &release_dir)?;
    }
    Ok(SeedResult {
        version: document.version.clone(),
        downloaded_files,
        reused_files,
        changed: true,
        activated: activate,
    })
}

fn materialize_stage(
    stage: &Path,
    cache: &Path,
    source: &SeedSource,
    index: &BundleIndex,
    raw_index: &[u8],
) -> Result<(usize, usize), String> {
    let mut downloaded = 0_usize;
    let mut reused = 0_usize;
    for entry in &index.files {
        let blob = cache.join(&entry.sha256);
        if verify_file(&blob, entry).is_err() {
            download_blob(source, entry, &blob)?;
            downloaded += 1;
        } else {
            reused += 1;
        }
        let destination = stage.join(&entry.path);
        let parent = destination
            .parent()
            .ok_or_else(|| "bundle path has no parent".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create bundle directory: {error}"))?;
        copy_blob(&blob, &destination)?;
        set_mode(&destination, entry.mode)?;
        verify_file(&destination, entry)?;
    }
    fs::write(stage.join(INDEX_NAME), raw_index)
        .map_err(|error| format!("cannot write bundle index: {error}"))?;
    Ok((downloaded, reused))
}

fn prime_cache_from_current(root: &Path, cache: &Path) -> Result<(), String> {
    let current = root.join("current");
    let Ok(raw_index) = fs::read(current.join(INDEX_NAME)) else {
        return Ok(());
    };
    let Ok(index_text) = std::str::from_utf8(&raw_index) else {
        return Ok(());
    };
    let Ok(index) = parse_index(index_text) else {
        return Ok(());
    };
    for entry in index.files {
        let source = current.join(&entry.path);
        let destination = cache.join(&entry.sha256);
        if !destination.exists() && verify_file(&source, &entry).is_ok() {
            cache_link_or_copy(&source, &destination)?;
        }
    }
    Ok(())
}

fn download_blob(
    source: &SeedSource,
    entry: &BundleFile,
    destination: &Path,
) -> Result<(), String> {
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()?));
    source.get_file(
        &format!("/v1/blob/{}", entry.sha256),
        &temporary,
        entry.bytes,
    )?;
    if let Err(error) = verify_file(&temporary, entry) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, destination)
        .map_err(|error| format!("cannot cache seeded blob: {error}"))
}

fn copy_blob(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("cannot materialize bundle file: {error}"))
}

fn verify_installed_release(
    release_dir: &Path,
    index: &BundleIndex,
    raw_index: &[u8],
    document: &ReleaseDocument,
) -> Result<(), String> {
    let installed = fs::read(release_dir.join(INDEX_NAME))
        .map_err(|error| format!("existing release is missing its index: {error}"))?;
    if installed != raw_index || sha256_bytes(&installed) != document.inventory_sha256 {
        return Err("existing release does not match the signed inventory".to_owned());
    }
    for entry in &index.files {
        verify_file(&release_dir.join(&entry.path), entry)?;
    }
    Ok(())
}

pub(crate) fn activate_release(root: &Path, release: &Path) -> Result<(), String> {
    let next = root.join("current.next");
    let current = root.join("current");
    let _ = fs::remove_file(&next);
    #[cfg(unix)]
    std::os::unix::fs::symlink(release, &next)
        .map_err(|error| format!("cannot create next release symlink: {error}"))?;
    #[cfg(not(unix))]
    return Err("seed activation requires a Unix host".to_owned());
    fs::rename(next, current).map_err(|error| format!("cannot activate seeded release: {error}"))
}

fn is_current_release(root: &Path, release: &Path) -> bool {
    fs::canonicalize(root.join("current")).ok() == fs::canonicalize(release).ok()
}
