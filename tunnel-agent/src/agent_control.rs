use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use reqwest::{blocking::Client, header::CONTENT_TYPE};
use serde::Deserialize;

use crate::{
    agent_lane::AgentLaneRole,
    agent_spec,
    manifest::{Config, NodeScope},
    release,
};

const MAX_CONTROL_KEY_BYTES: usize = 8_192;
const MAX_RELAY_RESPONSE_BYTES: usize = 1_048_576;
const MAX_RELAY_DOCUMENTS: usize = 64;

#[derive(Deserialize)]
struct ControlKeyResponse {
    private_key: String,
}

#[derive(Deserialize)]
struct RelayResponse {
    documents: Vec<RelayDocument>,
}

#[derive(Deserialize)]
struct RelayDocument {
    cluster_id: String,
    node_id: String,
    revision: u64,
    envelope: String,
}

#[derive(Deserialize)]
struct RelayHeartbeat {
    observed_at: u64,
}

pub(crate) fn prepare(
    config: &Config,
    state_dir: &Path,
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
) -> Result<(), String> {
    if !has_active_client(config) {
        return Ok(());
    }
    let mut keys = HashMap::new();
    for lane in active_clients(config) {
        let client_spec = lane.hpnssh.client.as_ref().expect("validated agent client");
        write_file(
            Path::new(&client_spec.known_hosts_file),
            &client_spec.known_host,
            0o644,
        )?;
        if !keys.contains_key(&lane.hpnssh.cluster_id) {
            let key = fetch_control_key(
                client,
                api_url,
                node_id,
                node_token,
                &lane.hpnssh.cluster_id,
            )?;
            keys.insert(lane.hpnssh.cluster_id.clone(), key);
        }
        let key = keys
            .get(&lane.hpnssh.cluster_id)
            .expect("inserted cluster control key");
        write_file(Path::new(&client_spec.identity_file), key, 0o600)?;
    }
    refresh_cache(config, state_dir, client, api_url, node_id, node_token)
}

pub(crate) fn refresh_cache(
    config: &Config,
    state_dir: &Path,
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
) -> Result<(), String> {
    if !has_active_client(config) {
        return Ok(());
    }
    let bundle_root = &config
        .bundle
        .as_ref()
        .ok_or_else(|| "foreign agent manifest has no bundle root".to_owned())?
        .root;
    let _ = release::cache_seed_release_document(
        api_url,
        node_id,
        node_token,
        bundle_root,
        &state_dir.join("release-document.json"),
    )?;
    refresh_relay_documents(
        client,
        api_url,
        node_id,
        node_token,
        &state_dir.join("relay-documents"),
    )?;
    report_relay_heartbeats(client, api_url, node_id, node_token, state_dir)
}

fn active_clients(config: &Config) -> impl Iterator<Item = &crate::agent_lane::AgentLane> {
    let selected: HashSet<_> = agent_spec::selected_lanes(&config.agent)
        .into_iter()
        .filter(|lane| lane.selected)
        .map(|lane| lane.id)
        .collect();
    config
        .agent
        .lanes
        .iter()
        .filter(move |lane| selected.contains(&lane.id) && lane.role == AgentLaneRole::Client)
}

fn has_active_client(config: &Config) -> bool {
    config.node_scope == NodeScope::Foreign && active_clients(config).next().is_some()
}

fn fetch_control_key(
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    cluster_id: &str,
) -> Result<String, String> {
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/clusters/{cluster_id}/control-key",
        api_url.trim_end_matches('/')
    );
    let response = client
        .get(endpoint)
        .bearer_auth(node_token)
        .send()
        .map_err(|error| format!("cannot fetch control key: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "control key request returned HTTP {}",
            response.status()
        ));
    }
    let body = response
        .text()
        .map_err(|error| format!("cannot read control key response: {error}"))?;
    let key = serde_json::from_str::<ControlKeyResponse>(&body)
        .map_err(|error| format!("cannot parse control key response: {error}"))?
        .private_key;
    if !key.starts_with("-----BEGIN PRIVATE KEY-----\n")
        || !key.ends_with("-----END PRIVATE KEY-----\n")
        || key.len() > MAX_CONTROL_KEY_BYTES
    {
        return Err("control key response is invalid".to_owned());
    }
    Ok(key)
}

fn refresh_relay_documents(
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    directory: &Path,
) -> Result<(), String> {
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/relay-documents",
        api_url.trim_end_matches('/')
    );
    let response = client
        .get(endpoint)
        .bearer_auth(node_token)
        .send()
        .map_err(|error| format!("cannot fetch relay documents: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "relay documents request returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELAY_RESPONSE_BYTES as u64)
    {
        return Err("relay documents response exceeds maximum size".to_owned());
    }
    let body = response
        .text()
        .map_err(|error| format!("cannot read relay documents response: {error}"))?;
    if body.len() > MAX_RELAY_RESPONSE_BYTES {
        return Err("relay documents response exceeds maximum size".to_owned());
    }
    let response: RelayResponse = serde_json::from_str(&body)
        .map_err(|error| format!("cannot parse relay documents response: {error}"))?;
    if response.documents.len() > MAX_RELAY_DOCUMENTS {
        return Err("relay documents response has too many entries".to_owned());
    }
    cache_relay_documents(directory, response.documents)
}

fn cache_relay_documents(directory: &Path, documents: Vec<RelayDocument>) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let mut active = HashSet::new();
    for document in documents {
        if !valid_uuid(&document.cluster_id)
            || !valid_uuid(&document.node_id)
            || document.revision == 0
            || document.envelope.is_empty()
            || document.envelope.len() > MAX_RELAY_RESPONSE_BYTES
        {
            return Err("relay documents response has an invalid entry".to_owned());
        }
        active.insert(document.node_id.clone());
        release::write_atomic(
            &directory.join(format!("{}.json", document.node_id)),
            document.envelope.as_bytes(),
        )?;
    }
    remove_stale_documents(directory, &active)
}

fn report_relay_heartbeats(
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    state_dir: &Path,
) -> Result<(), String> {
    let heartbeats = read_relay_heartbeats(&state_dir.join("relay-heartbeats"))?;
    let report_path = state_dir.join("relay-heartbeats-reported.json");
    let mut reported = read_reported_heartbeats(&report_path)?;
    let updates: Vec<_> = heartbeats
        .into_iter()
        .filter(|(domestic_id, observed_at)| {
            reported
                .get(domestic_id)
                .is_none_or(|seen| observed_at > seen)
        })
        .collect();
    if updates.is_empty() {
        return Ok(());
    }
    let observations: Vec<_> = updates
        .iter()
        .map(|(domestic_id, observed_at)| {
            serde_json::json!({ "node_id": domestic_id, "observed_at": observed_at })
        })
        .collect();
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/heartbeats",
        api_url.trim_end_matches('/')
    );
    let response = client
        .post(endpoint)
        .bearer_auth(node_token)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "observations": observations }).to_string())
        .send()
        .map_err(|error| format!("cannot report relay heartbeats: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "heartbeat report returned HTTP {}",
            response.status()
        ));
    }
    for (domestic_id, observed_at) in updates {
        reported.insert(domestic_id, observed_at);
    }
    let contents = serde_json::to_vec(&reported)
        .map_err(|error| format!("cannot serialize heartbeat report state: {error}"))?;
    release::write_atomic(&report_path, &contents)
}

fn read_relay_heartbeats(directory: &Path) -> Result<HashMap<String, u64>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(format!("cannot read relay heartbeats: {error}")),
    };
    let mut heartbeats = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read relay heartbeat entry: {error}"))?;
        let file_name = entry.file_name();
        let Some(node_id) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|value| valid_uuid(value))
            .map(str::to_owned)
        else {
            continue;
        };
        let contents = fs::read(entry.path())
            .map_err(|error| format!("cannot read relay heartbeat {node_id}: {error}"))?;
        let heartbeat = serde_json::from_slice::<RelayHeartbeat>(&contents)
            .map_err(|error| format!("cannot parse relay heartbeat {node_id}: {error}"))?;
        if heartbeat.observed_at > 0 {
            heartbeats.insert(node_id, heartbeat.observed_at);
        }
    }
    Ok(heartbeats)
}

fn read_reported_heartbeats(path: &Path) -> Result<HashMap<String, u64>, String> {
    match fs::read(path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("cannot parse heartbeat report state: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(format!("cannot read heartbeat report state: {error}")),
    }
}

fn remove_stale_documents(directory: &Path, active: &HashSet<String>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot read relay cache entry: {error}"))?;
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if valid_uuid(name) && !active.contains(name) {
            fs::remove_file(entry.path())
                .map_err(|error| format!("cannot remove stale relay document: {error}"))?;
        }
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str, mode: u32) -> Result<(), String> {
    release::write_atomic(path, contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("cannot secure {}: {error}", path.display()))?;
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            (matches!(index, 8 | 13 | 18 | 23) && byte == b'-') || byte.is_ascii_hexdigit()
        })
}
