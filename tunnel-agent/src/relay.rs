use std::{fs, path::Path};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;

use crate::{
    capsule,
    manifest::{self, NodeScope},
};

const MAX_ENVELOPE_BYTES: usize = 1_048_576;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    p: String,
    s: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    v: u8,
    c: String,
    n: String,
    i: u64,
    m: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayState {
    cluster_id: String,
}

pub(crate) struct VerifiedDocument {
    pub(crate) cluster_id: String,
    pub(crate) node_id: String,
    pub(crate) revision: u64,
    pub(crate) manifest_json: String,
}

pub(crate) fn verify_for_local_node(
    raw: &str,
    relay_state_path: &Path,
    node_id_path: &Path,
) -> Result<VerifiedDocument, String> {
    let state: RelayState = read_json(relay_state_path, "relay state")?;
    let node_id = read_text(node_id_path, "node id")?;
    verify_with_key(
        raw,
        &capsule::api_verifying_key()?,
        &state.cluster_id,
        &node_id,
    )
}

fn verify_with_key(
    raw: &str,
    key: &VerifyingKey,
    expected_cluster_id: &str,
    expected_node_id: &str,
) -> Result<VerifiedDocument, String> {
    if raw.len() > MAX_ENVELOPE_BYTES {
        return Err("relay document exceeds the maximum size".to_owned());
    }
    let envelope: Envelope = serde_json::from_str(raw)
        .map_err(|error| format!("cannot parse relay document envelope: {error}"))?;
    let payload = capsule::decode_url(&envelope.p, "relay document payload")?;
    let signature = capsule::decode_url(&envelope.s, "relay document signature")?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| "relay document signature is invalid".to_owned())?;
    key.verify(&payload, &signature)
        .map_err(|_| "relay document signature verification failed".to_owned())?;
    let payload: Payload = serde_json::from_slice(&payload)
        .map_err(|error| format!("cannot parse relay document payload: {error}"))?;
    if payload.v != 1 || !valid_uuid(&payload.c) || !valid_uuid(&payload.n) {
        return Err("relay document has an invalid version, cluster id, or node id".to_owned());
    }
    if payload.c != expected_cluster_id || payload.n != expected_node_id {
        return Err("relay document does not belong to this domestic node".to_owned());
    }
    let manifest = capsule::decode_url(&payload.m, "relay document manifest")?;
    if manifest.len() > MAX_ENVELOPE_BYTES {
        return Err("relay document manifest exceeds the maximum size".to_owned());
    }
    let manifest_json = String::from_utf8(manifest)
        .map_err(|_| "relay document manifest is not UTF-8".to_owned())?;
    let config = manifest::parse_config(&manifest_json)?;
    if config.version != 3 || config.node_scope != NodeScope::Domestic {
        return Err("relay document must contain a domestic version 3 manifest".to_owned());
    }
    Ok(VerifiedDocument {
        cluster_id: payload.c,
        node_id: payload.n,
        revision: payload.i,
        manifest_json,
    })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Result<T, String> {
    let value = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    serde_json::from_str(&value)
        .map_err(|error| format!("cannot parse {label} {}: {error}", path.display()))
}

fn read_text(path: &Path, label: &str) -> Result<String, String> {
    let value = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let value = value.trim().to_owned();
    if !valid_uuid(&value) {
        return Err(format!("{label} {} must be a UUID", path.display()));
    }
    Ok(value)
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            (matches!(index, 8 | 13 | 18 | 23) && byte == b'-') || byte.is_ascii_hexdigit()
        })
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    const CLUSTER_ID: &str = "4cd57b8f-8f97-4f80-8791-90bba10190d0";
    const NODE_ID: &str = "bfad2cfd-4856-4b78-b936-e861260d2d33";

    #[test]
    fn accepts_a_signed_domestic_document_for_the_expected_node() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let manifest = r#"{"version":3,"node_scope":"domestic","bundle":{"root":"/opt/tunnel-agent/current"},"tunnels":[]}"#;
        let payload = serde_json::json!({
            "v": 1, "c": CLUSTER_ID, "n": NODE_ID, "i": 2,
            "m": URL_SAFE_NO_PAD.encode(manifest),
        })
        .to_string();
        let signature = signing_key.sign(payload.as_bytes());
        let envelope = serde_json::json!({
            "p": URL_SAFE_NO_PAD.encode(payload),
            "s": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        })
        .to_string();
        let document =
            verify_with_key(&envelope, &signing_key.verifying_key(), CLUSTER_ID, NODE_ID)
                .expect("verified relay document");
        assert_eq!(document.revision, 2);
        assert_eq!(document.node_id, NODE_ID);
    }

    #[test]
    fn rejects_a_document_for_another_node() {
        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let payload = serde_json::json!({
            "v": 1, "c": CLUSTER_ID,
            "n": "d6caf939-b959-4471-918f-2e05ca282a37", "i": 2,
            "m": URL_SAFE_NO_PAD.encode("{}"),
        })
        .to_string();
        let envelope = serde_json::json!({
            "p": URL_SAFE_NO_PAD.encode(&payload),
            "s": URL_SAFE_NO_PAD.encode(signing_key.sign(payload.as_bytes()).to_bytes()),
        })
        .to_string();
        assert!(
            verify_with_key(&envelope, &signing_key.verifying_key(), CLUSTER_ID, NODE_ID).is_err()
        );
    }
}
