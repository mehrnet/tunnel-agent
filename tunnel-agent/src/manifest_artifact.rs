use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

use super::{ArtifactSpec, Config, NodeScope, PackageFormat, manifest_validate::validate_config};

pub fn artifact_binary_path(state_dir: &Path, artifact: &ArtifactSpec) -> PathBuf {
    state_dir
        .join("artifacts")
        .join(&artifact.id)
        .join(&artifact.version)
        .join(&artifact.binary)
}

pub fn sync_artifacts(
    config: &Config,
    state_dir: &Path,
    only_id: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    validate_config(config)?;
    if config.bundle.is_some() {
        return Err(
            "manifest version 3 uses preinstalled bundle artifacts; sync is intentionally disabled"
                .to_owned(),
        );
    }
    let mut paths = Vec::new();
    for artifact in &config.artifacts {
        if only_id.is_some_and(|id| id != artifact.id) {
            continue;
        }
        paths.push(sync_artifact(artifact, config.node_scope, state_dir)?);
    }
    if let Some(id) = only_id
        && !config.artifacts.iter().any(|artifact| artifact.id == id)
    {
        return Err(format!("unknown artifact {id:?}"));
    }
    Ok(paths)
}

fn sync_artifact(
    artifact: &ArtifactSpec,
    scope: NodeScope,
    state_dir: &Path,
) -> Result<PathBuf, String> {
    let destination = artifact_binary_path(state_dir, artifact);
    if artifact_is_installed(artifact, &destination)? {
        return Ok(destination);
    }
    let parent = destination
        .parent()
        .expect("artifact destination has parent");
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let cache_dir = state_dir.join("downloads");
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("cannot create {}: {error}", cache_dir.display()))?;
    let archive_path = cache_dir.join(format!("{}-{}.download", artifact.id, artifact.version));
    let mut sources = artifact
        .sources
        .iter()
        .filter(|source| scope.permits(source.scope))
        .collect::<Vec<_>>();
    sources.sort_by_key(|source| source.priority);
    if sources.is_empty() {
        return Err(format!(
            "artifact {:?} has no source compatible with node_scope",
            artifact.id
        ));
    }
    let mut failures = Vec::new();
    for source in sources {
        match download_to(&source.url, &archive_path) {
            Ok(()) => match install_download(artifact, &archive_path, &destination) {
                Ok(()) => return Ok(destination),
                Err(error) => failures.push(format!("{}: {error}", source.url)),
            },
            Err(error) => failures.push(format!("{}: {error}", source.url)),
        }
    }
    Err(format!(
        "artifact {:?} could not be synchronized: {}",
        artifact.id,
        failures.join("; ")
    ))
}

fn download_to(url: &str, destination: &Path) -> Result<(), String> {
    let temporary = destination.with_extension("part");
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|error| format!("cannot construct HTTPS client: {error}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|error| format!("download failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("download returned an error status: {error}"))?;
    let mut file = fs::File::create(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
    response
        .copy_to(&mut file)
        .map_err(|error| format!("cannot save download: {error}"))?;
    file.flush()
        .map_err(|error| format!("cannot flush download: {error}"))?;
    fs::rename(&temporary, destination)
        .map_err(|error| format!("cannot publish {}: {error}", destination.display()))?;
    Ok(())
}

pub(super) fn install_download(
    artifact: &ArtifactSpec,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let package_checksum = sha256_file(source)?;
    if package_checksum != artifact.sha256.to_ascii_lowercase() {
        return Err(format!(
            "package SHA-256 mismatch: expected {}, received {package_checksum}",
            artifact.sha256
        ));
    }
    let temporary = destination.with_extension("part");
    match artifact.package {
        PackageFormat::Binary => {
            let _ = fs::copy(source, &temporary)
                .map_err(|error| format!("cannot copy binary: {error}"))?;
        }
        PackageFormat::TarGz => extract_member(source, &artifact.binary, &temporary)?,
    };
    if let Some(binary_sha256) = &artifact.binary_sha256 {
        let binary_checksum = sha256_file(&temporary)?;
        if binary_checksum != binary_sha256.to_ascii_lowercase() {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "binary SHA-256 mismatch: expected {binary_sha256}, received {binary_checksum}"
            ));
        }
    }
    set_executable(&temporary)?;
    fs::rename(&temporary, destination)
        .map_err(|error| format!("cannot publish {}: {error}", destination.display()))?;
    Ok(())
}

fn artifact_is_installed(artifact: &ArtifactSpec, destination: &Path) -> Result<bool, String> {
    if !destination.is_file() {
        return Ok(false);
    }
    let expected = match artifact.package {
        PackageFormat::Binary => Some(artifact.sha256.as_str()),
        PackageFormat::TarGz => artifact.binary_sha256.as_deref(),
    };
    match expected {
        Some(expected) => Ok(sha256_file(destination)? == expected.to_ascii_lowercase()),
        // The id/version directory is immutable and publication occurred only after package verification.
        None => Ok(true),
    }
}

fn extract_member(archive_path: &Path, member: &Path, destination: &Path) -> Result<(), String> {
    let file =
        fs::File::open(archive_path).map_err(|error| format!("cannot open archive: {error}"))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|error| format!("cannot read archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("cannot read archive entry: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("cannot read archive path: {error}"))?;
        if path == member {
            let mut output = fs::File::create(destination)
                .map_err(|error| format!("cannot create extracted binary: {error}"))?;
            io::copy(&mut entry, &mut output)
                .map_err(|error| format!("cannot extract binary: {error}"))?;
            output
                .flush()
                .map_err(|error| format!("cannot flush extracted binary: {error}"))?;
            return Ok(());
        }
    }
    Err(format!("archive does not contain {}", member.display()))
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot make {} executable: {error}", path.display()))?;
    }
    Ok(())
}
