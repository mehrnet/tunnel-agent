use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;

use crate::manifest::{self, NodeScope};

const PREFIX: &str = "ta1";
const HEADER_WIDTH: usize = 5;
const BLOCK_WIDTH: usize = 7;
const BLOCK_BYTES: usize = 5;
const MAX_CAPSULE_BYTES: usize = 32_768;
const CAPSULE_PUBLIC_KEY: &str = "59O+lw4EIergZadHvVPK4RW688eIfIupFEBf0cdcTkc=";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    p: String,
    s: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capsule {
    v: u8,
    e: u64,
    n: String,
    c: String,
    i: u64,
    k: String,
    m: String,
    xv: String,
    xh: String,
    xu: String,
    av: String,
    ah: String,
    au: String,
    l: Vec<Relay>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Relay {
    h: String,
    p: u16,
}

pub(crate) struct Material {
    pub(crate) cluster_id: String,
    pub(crate) node_id: String,
    pub(crate) revision: u64,
}

pub(crate) fn install_domestic(
    raw: &str,
    etc_dir: &Path,
    state_dir: &Path,
    config_path: &Path,
) -> Result<Material, String> {
    let capsule = parse_capsule(raw)?;
    let manifest = decode_url(&capsule.m, "manifest")?;
    let manifest = String::from_utf8(manifest).map_err(|_| "manifest is not UTF-8".to_owned())?;
    let config = manifest::parse_config(&manifest)?;
    if config.version != 3 || config.node_scope != NodeScope::Domestic {
        return Err("domestic capsule must contain a domestic version 3 manifest".to_owned());
    }
    validate_bundle(&capsule.xv, &capsule.xh, &capsule.xu)?;
    validate_bundle(&capsule.av, &capsule.ah, &capsule.au)?;
    validate_ssh_public_key(&capsule.k)?;
    validate_relays(&capsule.l)?;

    write_private(&etc_dir.join("node-id"), &capsule.n)?;
    write_private(config_path, &manifest)?;
    write_private(
        &etc_dir.join("cluster-control-authorized_keys"),
        &format!(
            "ssh-ed25519 {} tunnel-agent-cluster-{}\n",
            capsule.k, capsule.c
        ),
    )?;
    let relay_state = serde_json::json!({
        "version": capsule.v,
        "cluster_id": capsule.c,
        "relays": capsule.l.iter().map(|relay| serde_json::json!({"host": relay.h, "port": relay.p})).collect::<Vec<_>>(),
    });
    write_private(
        &etc_dir.join("relay-state.json"),
        &serde_json::to_string(&relay_state).map_err(|error| error.to_string())?,
    )?;
    write_private(&state_dir.join("document-revision"), &capsule.i.to_string())?;

    Ok(Material {
        cluster_id: capsule.c,
        node_id: capsule.n,
        revision: capsule.i,
    })
}

fn parse_capsule(raw: &str) -> Result<Capsule, String> {
    let bytes = decode_base62(raw)?;
    let envelope: Envelope = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse capsule envelope: {error}"))?;
    let payload = decode_url(&envelope.p, "capsule payload")?;
    verify_signature(&payload, &decode_url(&envelope.s, "capsule signature")?)?;
    let capsule: Capsule = serde_json::from_slice(&payload)
        .map_err(|error| format!("cannot parse capsule payload: {error}"))?;
    validate_capsule(&capsule)?;
    Ok(capsule)
}

fn validate_capsule(capsule: &Capsule) -> Result<(), String> {
    if capsule.v != 1 || !valid_uuid(&capsule.n) || !valid_uuid(&capsule.c) {
        return Err("capsule has an invalid version, node id, or cluster id".to_owned());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system time is before the Unix epoch".to_owned())?
        .as_secs();
    if capsule.e <= now || capsule.e > now.saturating_add(604_800) {
        return Err("capsule has expired or exceeds the maximum lifetime".to_owned());
    }
    Ok(())
}

fn decode_base62(raw: &str) -> Result<Vec<u8>, String> {
    let body = raw
        .strip_prefix(PREFIX)
        .ok_or_else(|| "capsule has an invalid prefix".to_owned())?;
    if body.len() < HEADER_WIDTH
        || !body.is_ascii()
        || !(body.len() - HEADER_WIDTH).is_multiple_of(BLOCK_WIDTH)
    {
        return Err("capsule has an invalid base62 layout".to_owned());
    }
    let length = base62_number(&body[..HEADER_WIDTH])? as usize;
    if length == 0 || length > MAX_CAPSULE_BYTES {
        return Err("capsule has an invalid decoded length".to_owned());
    }
    let mut result = Vec::with_capacity((body.len() - HEADER_WIDTH) / BLOCK_WIDTH * BLOCK_BYTES);
    let (blocks, remainder) = body.as_bytes()[HEADER_WIDTH..].as_chunks::<BLOCK_WIDTH>();
    debug_assert!(remainder.is_empty());
    for block in blocks {
        let value = base62_number(std::str::from_utf8(block).map_err(|_| "invalid base62 block")?)?;
        if value > 0xFF_FF_FF_FF_FF {
            return Err("capsule block exceeds five bytes".to_owned());
        }
        for offset in (0..BLOCK_BYTES).rev() {
            result.push(((value >> (offset * 8)) & 0xFF) as u8);
        }
    }
    if result.len() < length || result[length..].iter().any(|byte| *byte != 0) {
        return Err("capsule has invalid padding".to_owned());
    }
    result.truncate(length);
    Ok(result)
}

fn base62_number(value: &str) -> Result<u64, String> {
    value.bytes().try_fold(0_u64, |result, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' => byte - b'A' + 10,
            b'a'..=b'z' => byte - b'a' + 36,
            _ => return Err("capsule includes a non-base62 character".to_owned()),
        };
        result
            .checked_mul(62)
            .and_then(|number| number.checked_add(u64::from(digit)))
            .ok_or_else(|| "capsule base62 value overflows".to_owned())
    })
}

fn decode_url(value: &str, label: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| format!("{label} is not valid base64url"))
}

fn verify_signature(payload: &[u8], signature: &[u8]) -> Result<(), String> {
    let public = STANDARD
        .decode(CAPSULE_PUBLIC_KEY)
        .map_err(|_| "compiled capsule public key is invalid".to_owned())?;
    let public: [u8; 32] = public
        .try_into()
        .map_err(|_| "compiled capsule key has invalid length")?;
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|_| "compiled capsule key is invalid".to_owned())?;
    let signature =
        Signature::from_slice(signature).map_err(|_| "capsule signature is invalid".to_owned())?;
    key.verify(payload, &signature)
        .map_err(|_| "capsule signature verification failed".to_owned())
}

fn validate_bundle(version: &str, sha256: &str, urls: &str) -> Result<(), String> {
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        return Err("capsule bundle version is invalid".to_owned());
    }
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("capsule bundle SHA-256 is invalid".to_owned());
    }
    if urls.split(',').next().is_none_or(str::is_empty)
        || urls.split(',').any(|url| !valid_url(url))
    {
        return Err("capsule bundle mirror URL is invalid".to_owned());
    }
    Ok(())
}

fn validate_ssh_public_key(value: &str) -> Result<(), String> {
    let key = STANDARD
        .decode(value)
        .map_err(|_| "cluster SSH public key is invalid base64".to_owned())?;
    if key.len() != 51
        || key[..4] != [0, 0, 0, 11]
        || &key[4..15] != b"ssh-ed25519"
        || key[15..19] != [0, 0, 0, 32]
    {
        return Err("cluster SSH public key is not an Ed25519 SSH key".to_owned());
    }
    Ok(())
}

fn validate_relays(relays: &[Relay]) -> Result<(), String> {
    if relays.is_empty()
        || relays.len() > 16
        || relays
            .iter()
            .any(|relay| relay.p == 0 || !valid_host(&relay.h))
    {
        return Err("capsule has invalid relay endpoints".to_owned());
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            (matches!(index, 8 | 13 | 18 | 23) && byte == b'-') || byte.is_ascii_hexdigit()
        })
}

fn valid_url(value: &str) -> bool {
    value.starts_with("https://")
        && !value.is_empty()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b',' | b'\\' | b'#' | b'?'))
}

fn valid_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
}

fn write_private(path: &Path, value: &str) -> Result<(), String> {
    manifest::write_config_atomic(path, value)?;
    #[cfg(unix)]
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .map_err(|error| format!("cannot secure {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_base62(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        fn block(value: u64, width: usize) -> String {
            let mut output = vec![b'0'; width];
            let mut value = value;
            for index in (0..width).rev() {
                output[index] = ALPHABET[(value % 62) as usize];
                value /= 62;
            }
            String::from_utf8(output).expect("base62 is UTF-8")
        }
        let mut output = format!("{PREFIX}{}", block(bytes.len() as u64, HEADER_WIDTH));
        for chunk in bytes.chunks(BLOCK_BYTES) {
            let value = chunk
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte))
                << ((BLOCK_BYTES - chunk.len()) * 8);
            output.push_str(&block(value, BLOCK_WIDTH));
        }
        output
    }

    #[test]
    fn base62_round_trip_preserves_short_final_blocks() {
        let raw = br#"{\"v\":1,\"n\":\"capsule\"}"#;
        assert_eq!(decode_base62(&encode_base62(raw)).expect("decode"), raw);
    }

    #[test]
    fn rejects_nonzero_base62_padding() {
        let mut capsule = encode_base62(b"ok");
        let last = capsule.pop().expect("last character");
        capsule.push(if last == 'z' { 'y' } else { 'z' });
        assert!(decode_base62(&capsule).is_err());
    }
}
