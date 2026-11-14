use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::{Component, Path},
    time::{SystemTime, UNIX_EPOCH},
};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::{blocking::Client, redirect::Policy};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::capsule;

pub(crate) const INDEX_NAME: &str = "bundle-index.json";
const MAX_DOCUMENT_BYTES: usize = 16_384;
const MAX_INDEX_BYTES: usize = 1_048_576;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    p: String,
    s: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleasePayload {
    v: u8,
    a: String,
    b: String,
    h: String,
    i: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseResponse {
    envelope: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleIndex {
    format: u8,
    pub(crate) version: String,
    pub(crate) architecture: String,
    pub(crate) files: Vec<BundleFile>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleFile {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
    pub(crate) mode: u32,
}

pub(crate) struct ReleaseDocument {
    pub(crate) architecture: String,
    pub(crate) version: String,
    pub(crate) inventory_sha256: String,
}

pub(crate) fn cache_release_document(
    api_url: &str,
    node_id: &str,
    node_token: &str,
    output: &Path,
) -> Result<(), String> {
    let architecture = local_architecture()?;
    validate_credentials(node_id, node_token)?;
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/bundle-release?architecture={architecture}",
        api_url.trim_end_matches('/')
    );
    if !endpoint.starts_with("https://") {
        return Err("bundle-release API URL must use HTTPS".to_owned());
    }
    let response = Client::builder()
        .redirect(Policy::none())
        .build()
        .map_err(|error| format!("cannot build release client: {error}"))?
        .get(endpoint)
        .bearer_auth(node_token)
        .send()
        .map_err(|error| format!("cannot fetch signed release document: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "bundle-release API returned HTTP {}",
            response.status()
        ));
    }
    let body = response
        .text()
        .map_err(|error| format!("cannot read signed release document: {error}"))?;
    let document = serde_json::from_str::<ReleaseResponse>(&body)
        .map_err(|error| format!("cannot parse signed release response: {error}"))?;
    verify_release_document(&document.envelope, &architecture)?;
    write_atomic(output, document.envelope.as_bytes())
}

pub(crate) fn parse_index(raw: &str) -> Result<BundleIndex, String> {
    if raw.len() > MAX_INDEX_BYTES {
        return Err("bundle inventory exceeds the maximum size".to_owned());
    }
    let index: BundleIndex = serde_json::from_str(raw)
        .map_err(|error| format!("cannot parse bundle inventory: {error}"))?;
    if index.format != 1
        || !valid_version(&index.version)
        || !valid_architecture(&index.architecture)
    {
        return Err("bundle inventory has invalid metadata".to_owned());
    }
    if index.files.is_empty() || index.files.len() > 1_024 {
        return Err("bundle inventory has an invalid file count".to_owned());
    }
    let mut paths = HashSet::new();
    for file in &index.files {
        if !valid_bundle_path(&file.path)
            || !valid_hash(&file.sha256)
            || file.bytes > MAX_FILE_BYTES
            || file.mode == 0
            || file.mode & !0o777 != 0
            || !paths.insert(&file.path)
        {
            return Err("bundle inventory contains an invalid file".to_owned());
        }
    }
    Ok(index)
}

pub(crate) fn verify_release_document(
    raw: &str,
    architecture: &str,
) -> Result<ReleaseDocument, String> {
    verify_release_document_with_key(raw, architecture, &capsule::api_verifying_key()?)
}

fn verify_release_document_with_key(
    raw: &str,
    architecture: &str,
    key: &VerifyingKey,
) -> Result<ReleaseDocument, String> {
    if raw.len() > MAX_DOCUMENT_BYTES {
        return Err("signed release document exceeds the maximum size".to_owned());
    }
    let envelope: Envelope = serde_json::from_str(raw)
        .map_err(|error| format!("cannot parse signed release document: {error}"))?;
    let payload = capsule::decode_url(&envelope.p, "release document payload")?;
    let signature = capsule::decode_url(&envelope.s, "release document signature")?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| "release document signature is invalid".to_owned())?;
    key.verify(&payload, &signature)
        .map_err(|_| "release document signature verification failed".to_owned())?;
    let payload: ReleasePayload = serde_json::from_slice(&payload)
        .map_err(|error| format!("cannot parse release document payload: {error}"))?;
    if payload.v != 1
        || payload.a != architecture
        || !valid_version(&payload.b)
        || !valid_hash(&payload.h)
        || !valid_hash(&payload.i)
    {
        return Err("release document has invalid metadata".to_owned());
    }
    Ok(ReleaseDocument {
        architecture: payload.a,
        version: payload.b,
        inventory_sha256: payload.i,
    })
}

pub(crate) fn ensure_index_matches_document(
    index: &BundleIndex,
    document: &ReleaseDocument,
) -> Result<(), String> {
    if index.version == document.version && index.architecture == document.architecture {
        Ok(())
    } else {
        Err("bundle inventory does not match the signed release document".to_owned())
    }
}

pub(crate) fn verify_file(path: &Path, entry: &BundleFile) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() != entry.bytes || sha256_file(path)? != entry.sha256 {
        return Err(format!(
            "bundle file verification failed: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte as char).is_ascii_lowercase() && byte.is_ascii_hexdigit()
        })
}

pub(crate) fn local_architecture() -> Result<String, String> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64".to_owned()),
        "aarch64" => Ok("arm64".to_owned()),
        value => Err(format!("unsupported local architecture: {value}")),
    }
}

pub(crate) fn as_utf8<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str, String> {
    std::str::from_utf8(bytes).map_err(|_| format!("{label} is not UTF-8"))
}

pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension(format!("next-{}", unique_suffix()?));
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(temporary, path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))
}

pub(crate) fn unique_suffix() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system time is before the Unix epoch".to_owned())?
        .as_nanos();
    Ok(format!("{}-{nanos}", std::process::id()))
}

pub(crate) fn cache_link_or_copy(source: &Path, destination: &Path) -> Result<(), String> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| format!("cannot copy bundle file: {error}")),
    }
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("cannot set bundle file mode: {error}"))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_: &Path, _: u32) -> Result<(), String> {
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32_768];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn valid_bundle_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn valid_architecture(value: &str) -> bool {
    matches!(value, "amd64" | "arm64")
}

fn validate_credentials(node_id: &str, token: &str) -> Result<(), String> {
    let valid_id = node_id.len() == 36
        && node_id.bytes().enumerate().all(|(index, byte)| {
            (matches!(index, 8 | 13 | 18 | 23) && byte == b'-') || byte.is_ascii_hexdigit()
        });
    if valid_id && token.starts_with(&format!("{node_id}.")) && token.len() > node_id.len() + 1 {
        Ok(())
    } else {
        Err("node credentials are invalid".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn rejects_inventory_path_traversal() {
        let raw = r#"{"format":1,"version":"v1","architecture":"amd64","files":[{"path":"../agent","sha256":"0000000000000000000000000000000000000000000000000000000000000000","bytes":1,"mode":493}]}"#;
        assert!(parse_index(raw).is_err());
    }

    #[test]
    fn verifies_a_signed_release_for_the_local_architecture() {
        let key = SigningKey::from_bytes(&[8; 32]);
        let payload = r#"{"v":1,"a":"amd64","b":"v1","h":"0000000000000000000000000000000000000000000000000000000000000000","i":"1111111111111111111111111111111111111111111111111111111111111111"}"#;
        let envelope = serde_json::json!({
            "p": URL_SAFE_NO_PAD.encode(payload),
            "s": URL_SAFE_NO_PAD.encode(key.sign(payload.as_bytes()).to_bytes()),
        });
        let release =
            verify_release_document_with_key(&envelope.to_string(), "amd64", &key.verifying_key())
                .expect("release document");
        assert_eq!(release.version, "v1");
    }
}
