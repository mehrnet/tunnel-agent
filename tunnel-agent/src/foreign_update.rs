use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

use flate2::read::GzDecoder;
use reqwest::{blocking::Client, redirect::Policy};

use crate::{
    agent::AgentSettings,
    manifest::Config,
    release::{self, INDEX_NAME, ReleaseDocument},
    seed,
};

const PUBLIC_ORIGIN: &str = "https://tunnel-files.mehrnet.com";
const ARCHIVE_LIMIT: u64 = 256 * 1024 * 1024;
const PART_LIMIT: u64 = 25 * 1024 * 1024;
const EXTRACT_LIMIT: u64 = 512 * 1024 * 1024;

struct Part<'a> {
    sha: &'a str,
    bytes: u64,
    name: &'a str,
}

fn parse_parts<'a>(raw: &'a str, archive: &str, digest: &str) -> Result<Vec<Part<'a>>, String> {
    let mut lines = raw.lines();
    if lines.next() != Some("TUNNEL_AGENT_BUNDLE_PARTS=1")
        || lines
            .next()
            .and_then(|line| line.strip_prefix("ARCHIVE_SHA256="))
            != Some(digest)
    {
        return Err("chunk manifest does not match the signed archive".to_owned());
    }
    let size = lines
        .next()
        .and_then(|line| line.strip_prefix("ARCHIVE_BYTES="))
        .and_then(|size| size.parse::<u64>().ok())
        .filter(|size| (1..=ARCHIVE_LIMIT).contains(size))
        .ok_or("invalid archive size")?;
    let mut parts = Vec::new();
    let mut sum = 0;
    for (index, line) in lines.enumerate() {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        let ["PART", hash, bytes, name] = fields.as_slice() else {
            return Err("invalid archive chunk".to_owned());
        };
        if index >= 64
            || !release::valid_hash(hash)
            || *name != format!("{archive}.part-{index:04}")
        {
            return Err("invalid archive chunk identity".to_owned());
        }
        let bytes = bytes
            .parse::<u64>()
            .ok()
            .filter(|bytes| (1..=PART_LIMIT).contains(bytes))
            .ok_or("invalid archive chunk size")?;
        sum += bytes;
        parts.push(Part {
            sha: hash,
            bytes,
            name,
        });
    }
    if sum != size {
        return Err("archive chunks do not match the declared size".to_owned());
    }
    Ok(parts)
}

fn download(client: &Client, url: &str, cap: u64, out: &mut impl Write) -> Result<u64, String> {
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|err| format!("release file fetch failed: {err}"))?;
    let bytes = io::copy(&mut response.take(cap + 1), out)
        .map_err(|err| format!("release file download failed: {err}"))?;
    if bytes > cap {
        return Err("release file exceeds its size limit".to_owned());
    }
    Ok(bytes)
}

fn download_archive(root: &Path, dest: &Path, doc: &ReleaseDocument) -> Result<(), String> {
    let arch = if doc.architecture == "amd64" {
        "x86_64"
    } else {
        "aarch64"
    };
    let archive = format!(
        "tunnel-agent-{}-{arch}-unknown-linux-musl.tar.gz",
        doc.version
    );
    let base = format!("{PUBLIC_ORIGIN}/bundles/{}", doc.version);
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| err.to_string())?;
    let mut manifest = Vec::new();
    download(
        &client,
        &format!("{base}/{archive}.parts"),
        32_768,
        &mut manifest,
    )?;
    let chunks = parse_parts(
        release::as_utf8(&manifest, "chunk manifest")?,
        &archive,
        &doc.archive_sha256,
    )?;
    let cache = root.join("cache").join("archive-parts");
    fs::create_dir_all(&cache).map_err(|err| err.to_string())?;
    let mut output = File::create(dest).map_err(|err| err.to_string())?;
    for part in chunks {
        let path = cache_part(&client, &base, &cache, &part)?;
        io::copy(
            &mut File::open(path).map_err(|err| err.to_string())?,
            &mut output,
        )
        .map_err(|err| err.to_string())?;
    }
    drop(output);
    if release::sha256_file(dest)? != doc.archive_sha256 {
        return Err("assembled archive failed signature-bound digest verification".to_owned());
    }
    Ok(())
}

fn cache_part(
    client: &Client,
    base: &str,
    cache: &Path,
    part: &Part<'_>,
) -> Result<PathBuf, String> {
    let dest = cache.join(part.sha);
    if fs::metadata(&dest).is_ok_and(|meta| meta.len() == part.bytes)
        && release::sha256_file(&dest).is_ok_and(|hash| hash == part.sha)
    {
        return Ok(dest);
    }
    let stage = dest.with_extension(release::unique_suffix()?);
    let result = (|| {
        let mut file = File::create(&stage).map_err(|err| err.to_string())?;
        let bytes = download(
            client,
            &format!("{base}/{}", part.name),
            part.bytes,
            &mut file,
        )?;
        drop(file);
        if bytes != part.bytes || release::sha256_file(&stage)? != part.sha {
            return Err("archive chunk failed digest verification".to_owned());
        }
        fs::rename(&stage, &dest).map_err(|err| err.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result.map(|()| dest)
}

fn unpack(archive: &Path, stage: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|err| err.to_string())?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    let mut paths = HashSet::new();
    let mut bytes = 0_u64;
    for entry in tar.entries().map_err(|err| err.to_string())? {
        let mut entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path().map_err(|err| err.to_string())?.into_owned();
        let kind = entry.header().entry_type();
        if !path.starts_with("tunnel-agent")
            || path
                .components()
                .any(|item| !matches!(item, Component::Normal(_)))
            || !(kind.is_file() || kind.is_dir())
            || !paths.insert(path)
            || paths.len() > 4096
        {
            return Err("unsafe entry in release archive".to_owned());
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > EXTRACT_LIMIT || !entry.unpack_in(stage).map_err(|err| err.to_string())? {
            return Err("release archive exceeds extraction limits or escapes stage".to_owned());
        }
    }
    Ok(())
}

fn verify_candidate(
    candidate: &Path,
    doc: &ReleaseDocument,
    config: &Path,
    state: &Path,
) -> Result<(), String> {
    let raw = fs::read(candidate.join(INDEX_NAME)).map_err(|err| err.to_string())?;
    if release::sha256_bytes(&raw) != doc.inventory_sha256 {
        return Err("candidate does not match signed inventory".to_owned());
    }
    let index = release::parse_index(release::as_utf8(&raw, "bundle index")?)?;
    release::ensure_index_matches_document(&index, doc)?;
    for file in &index.files {
        release::verify_file(&candidate.join(&file.path), file)?;
    }
    if !index
        .files
        .iter()
        .any(|entry| entry.path == "bin/tunnel-agent" && entry.mode & 0o100 != 0)
    {
        return Err("candidate does not index an executable tunnel-agent".to_owned());
    }
    let executed = Command::new(candidate.join("bin/tunnel-agent"))
        .arg("validate")
        .arg("--config")
        .arg(config)
        .arg("--state-dir")
        .arg(state)
        .output()
        .map_err(|err| format!("cannot execute candidate: {err}"))?;
    if !executed.status.success() {
        return Err("candidate failed local manifest validation".to_owned());
    }
    Ok(())
}

fn apply(
    root: &Path,
    config: &Path,
    state: &Path,
    credentials: (&str, &str, &str),
) -> Result<Option<String>, String> {
    let (api, id, token) = credentials;
    let raw = release::fetch_release_document(api, id, token)?;
    let doc = release::verify_release_document(&raw, &release::local_architecture()?)?;
    let current = root.join("current");
    let index = fs::read(current.join(INDEX_NAME)).map_err(|err| err.to_string())?;
    if release::sha256_bytes(&index) == doc.inventory_sha256 {
        return Ok(None);
    }
    if fs::canonicalize(current.join("bin/tunnel-agent")).ok() != std::env::current_exe().ok() {
        return Err(
            "automatic foreign updates require the running binary to be installed under current"
                .to_owned(),
        );
    }
    let releases = root.join("releases");
    let candidate = releases.join(&doc.version);
    if !candidate.exists() {
        let stage = releases.join(format!(".foreign-{}", release::unique_suffix()?));
        fs::create_dir(&stage).map_err(|err| err.to_string())?;
        let result = (|| {
            let archive = stage.join("bundle.tar.gz");
            download_archive(root, &archive, &doc)?;
            unpack(&archive, &stage)?;
            let unpacked = stage.join("tunnel-agent");
            verify_candidate(&unpacked, &doc, config, state)?;
            fs::rename(unpacked, &candidate).map_err(|err| err.to_string())
        })();
        let _ = fs::remove_dir_all(stage);
        result?;
    } else {
        verify_candidate(&candidate, &doc, config, state)?;
    }
    seed::activate_release(root, &candidate)?;
    Ok(Some(doc.version))
}

type UpdateResult = Result<Option<String>, String>;

pub(crate) struct Poller(Option<tokio::task::JoinHandle<UpdateResult>>);

impl Poller {
    pub(crate) fn new() -> Self {
        Self(None)
    }

    pub(crate) fn start(
        &mut self,
        settings: &AgentSettings,
        config: &Config,
        credentials: (&str, &str, &str),
    ) {
        if self.0.is_some() {
            return;
        }
        let Some(root) = config
            .bundle
            .as_ref()
            .and_then(|bundle| bundle.root.parent())
        else {
            return;
        };
        let (root, config, state) = (
            root.to_path_buf(),
            settings.config_path.clone(),
            settings.state_dir.clone(),
        );
        let (api, id, token) = credentials;
        let (api, id, token) = (api.to_owned(), id.to_owned(), token.to_owned());
        self.0 = Some(tokio::task::spawn_blocking(move || {
            apply(&root, &config, &state, (&api, &id, &token))
        }));
    }

    pub(crate) async fn result(&mut self) -> UpdateResult {
        let Some(task) = &mut self.0 else {
            return std::future::pending().await;
        };
        let result = task
            .await
            .map_err(|err| format!("foreign update task failed: {err}"));
        self.0 = None;
        result?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_chunk_identity_size_and_signed_archive_digest() {
        let digest = "a".repeat(64);
        let part = "b".repeat(64);
        let fixture = format!(
            "TUNNEL_AGENT_BUNDLE_PARTS=1\nARCHIVE_SHA256={digest}\nARCHIVE_BYTES=10\nPART {part} 4 agent.part-0000\nPART {part} 6 agent.part-0001\n"
        );
        assert_eq!(parse_parts(&fixture, "agent", &digest).unwrap().len(), 2);
        assert!(parse_parts(&fixture, "agent", &part).is_err());
        assert!(parse_parts(&fixture.replace("0001", "0000"), "agent", &digest).is_err());
        assert!(parse_parts(&fixture.replace("BYTES=10", "BYTES=11"), "agent", &digest).is_err());
    }

    #[test]
    #[ignore = "requires a published signed release envelope and the public files host"]
    fn published_release_download_and_stage() {
        let envelope = fs::read_to_string(std::env::var("TUNNEL_TEST_ENVELOPE").unwrap()).unwrap();
        let doc =
            release::verify_release_document(&envelope, &release::local_architecture().unwrap())
                .unwrap();
        let root = std::env::temp_dir().join(release::unique_suffix().unwrap());
        fs::create_dir(&root).unwrap();
        let archive = root.join("bundle.tar.gz");
        let result = (|| -> Result<(), String> {
            download_archive(&root, &archive, &doc)?;
            unpack(&archive, &root)?;
            let config = root.join("manifest.json");
            fs::write(&config, r#"{"version":3,"node_scope":"foreign","bundle":{"root":"/opt/tunnel-agent/current"}}"#)
                .map_err(|err| err.to_string())?;
            verify_candidate(&root.join("tunnel-agent"), &doc, &config, &root)?;
            Ok(())
        })();
        fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }
}
