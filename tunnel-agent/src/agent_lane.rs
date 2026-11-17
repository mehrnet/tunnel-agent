use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

use crate::{
    agent_spec::{AgentDirection, AgentListener, AgentPeer, AgentTransport},
    identity::valid_uuid,
    manifest::{GeneratedFile, ListenerClaim, NodeScope, ProcessSpec, RestartPolicy},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentLane {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub transport: AgentTransport,
    pub direction: AgentDirection,
    pub role: AgentLaneRole,
    pub priority: u16,
    pub peer: AgentPeer,
    pub listener: AgentListener,
    pub hpnssh: HpnSshLane,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentLaneRole {
    Server,
    Client,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HpnSshLane {
    pub cluster_id: String,
    pub server: Option<HpnSshServer>,
    pub client: Option<HpnSshClient>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HpnSshServer {
    pub bind: String,
    pub port: u16,
    pub host_key_file: String,
    pub authorized_keys_file: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HpnSshClient {
    pub user: String,
    pub identity_file: String,
    pub known_hosts_file: String,
    /// Exact OpenSSH known_hosts entry for the dedicated management listener.
    pub known_host: String,
    pub target: AgentTarget,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct AgentLanePlan {
    pub id: String,
    pub selected: bool,
    pub transport: AgentTransport,
    pub direction: AgentDirection,
    pub role: AgentLaneRole,
    pub priority: u16,
    pub peer: AgentPeer,
    pub listener: AgentListener,
}

pub fn validate(lane: &AgentLane, scope: NodeScope) -> Result<(), String> {
    validate_id(&lane.id)?;
    validate_peer(&lane.peer, &lane.id)?;
    validate_listener(&lane.listener, &lane.id)?;
    validate_restart(&lane.restart, &lane.id)?;
    if lane.transport != AgentTransport::HpnSsh {
        return Err(format!(
            "agent lane {:?} transport is not implemented",
            lane.id
        ));
    }
    if lane.direction != AgentDirection::ForeignToDomestic {
        return Err(format!(
            "agent lane {:?} must use foreign_to_domestic",
            lane.id
        ));
    }
    if !valid_uuid(&lane.hpnssh.cluster_id) {
        return Err(format!(
            "agent lane {:?} cluster_id must be a UUID",
            lane.id
        ));
    }
    match (scope, lane.role) {
        (NodeScope::Domestic, AgentLaneRole::Server) => validate_server(lane),
        (NodeScope::Foreign, AgentLaneRole::Client) => validate_client(lane),
        _ => Err(format!(
            "agent lane {:?} role does not match node_scope",
            lane.id
        )),
    }
}

pub fn compile(
    lane: &AgentLane,
    selected: bool,
    bundle_root: &Path,
    state_dir: &Path,
) -> (
    AgentLanePlan,
    Vec<ProcessSpec>,
    Vec<GeneratedFile>,
    Vec<ListenerClaim>,
) {
    let plan = AgentLanePlan {
        id: lane.id.clone(),
        selected,
        transport: lane.transport,
        direction: lane.direction,
        role: lane.role,
        priority: lane.priority,
        peer: lane.peer.clone(),
        listener: lane.listener.clone(),
    };
    if !selected {
        return (plan, Vec::new(), Vec::new(), Vec::new());
    }
    match lane.role {
        AgentLaneRole::Server => compile_server(lane, bundle_root, state_dir, plan),
        AgentLaneRole::Client => compile_client(lane, bundle_root, state_dir, plan),
    }
}

fn compile_server(
    lane: &AgentLane,
    bundle_root: &Path,
    state_dir: &Path,
    plan: AgentLanePlan,
) -> (
    AgentLanePlan,
    Vec<ProcessSpec>,
    Vec<GeneratedFile>,
    Vec<ListenerClaim>,
) {
    let server = lane.hpnssh.server.as_ref().expect("validated agent server");
    let config_path = state_dir
        .join("generated")
        .join(format!("agent-lane-{}.sshd_config", lane.id));
    let process = ProcessSpec {
        id: format!("agent-lane-{}", lane.id),
        argv: vec![
            bundle_root
                .join("hpnssh/sbin/hpnsshd")
                .display()
                .to_string(),
            "-D".to_owned(),
            "-f".to_owned(),
            config_path.display().to_string(),
        ],
        env: BTreeMap::new(),
        workdir: None,
        restart: lane.restart.clone(),
    };
    let generated = GeneratedFile {
        path: config_path,
        content: render_server_config(server),
    };
    let listeners = vec![
        ListenerClaim {
            owner: format!("agent lane {} server", lane.id),
            bind: server.bind.clone(),
            port: server.port,
        },
        ListenerClaim {
            owner: format!("agent lane {} mapped seed", lane.id),
            bind: lane.listener.bind.clone(),
            port: lane.listener.port,
        },
    ];
    (plan, vec![process], vec![generated], listeners)
}

fn compile_client(
    lane: &AgentLane,
    bundle_root: &Path,
    state_dir: &Path,
    plan: AgentLanePlan,
) -> (
    AgentLanePlan,
    Vec<ProcessSpec>,
    Vec<GeneratedFile>,
    Vec<ListenerClaim>,
) {
    let client = lane.hpnssh.client.as_ref().expect("validated agent client");
    let forward = format!(
        "{}:{}:{}:{}",
        lane.listener.bind, lane.listener.port, client.target.host, client.target.port
    );
    let tunnel = ProcessSpec {
        id: format!("agent-lane-{}", lane.id),
        argv: vec![
            bundle_root.join("hpnssh/bin/hpnssh").display().to_string(),
            "-N".to_owned(),
            "-o".to_owned(),
            "ExitOnForwardFailure=yes".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=20".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=3".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=yes".to_owned(),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", client.known_hosts_file),
            "-i".to_owned(),
            client.identity_file.clone(),
            "-p".to_owned(),
            lane.peer.port.to_string(),
            "-R".to_owned(),
            forward,
            format!("{}@{}", client.user, lane.peer.host),
        ],
        env: BTreeMap::new(),
        workdir: None,
        restart: lane.restart.clone(),
    };
    let seed = ProcessSpec {
        id: format!("agent-seed-{}", lane.id),
        argv: vec![
            bundle_root.join("bin/tunnel-agent").display().to_string(),
            "seed-serve".to_owned(),
            "--root".to_owned(),
            bundle_root
                .parent()
                .expect("bundle root has parent")
                .display()
                .to_string(),
            "--listen".to_owned(),
            format!("{}:{}", client.target.host, client.target.port),
            "--release-document".to_owned(),
            state_dir
                .join("release-document.json")
                .display()
                .to_string(),
            "--relay-directory".to_owned(),
            state_dir.join("relay-documents").display().to_string(),
        ],
        env: BTreeMap::new(),
        workdir: None,
        restart: lane.restart.clone(),
    };
    let listeners = vec![ListenerClaim {
        owner: format!("agent seed {}", lane.id),
        bind: client.target.host.clone(),
        port: client.target.port,
    }];
    (plan, vec![tunnel, seed], Vec::new(), listeners)
}

fn render_server_config(server: &HpnSshServer) -> String {
    format!(
        "Port {}\nListenAddress {}\nHostKey {}\nAuthorizedKeysFile {}\nPermitRootLogin prohibit-password\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nChallengeResponseAuthentication no\nAllowTcpForwarding remote\nGatewayPorts no\nX11Forwarding no\nAllowAgentForwarding no\nPermitTTY no\nUseDNS no\nClientAliveInterval 20\nClientAliveCountMax 3\nMaxSessions 8\n",
        server.port, server.bind, server.host_key_file, server.authorized_keys_file
    )
}

fn validate_server(lane: &AgentLane) -> Result<(), String> {
    let Some(server) = &lane.hpnssh.server else {
        return Err(format!(
            "agent lane {:?} server settings are required",
            lane.id
        ));
    };
    if lane.hpnssh.client.is_some()
        || !matches!(server.bind.as_str(), "0.0.0.0" | "::")
        || server.port < 1024
        || server.host_key_file.is_empty()
        || server.authorized_keys_file.is_empty()
    {
        return Err(format!(
            "agent lane {:?} has invalid server settings",
            lane.id
        ));
    }
    Ok(())
}

fn validate_client(lane: &AgentLane) -> Result<(), String> {
    let Some(client) = &lane.hpnssh.client else {
        return Err(format!(
            "agent lane {:?} client settings are required",
            lane.id
        ));
    };
    if lane.hpnssh.server.is_some()
        || client.user.is_empty()
        || client.identity_file.is_empty()
        || client.known_hosts_file.is_empty()
        || !valid_known_host(&client.known_host, &lane.peer)
        || !matches!(client.target.host.as_str(), "127.0.0.1" | "::1")
        || client.target.port < 1024
    {
        return Err(format!(
            "agent lane {:?} has invalid client settings",
            lane.id
        ));
    }
    Ok(())
}

fn valid_known_host(value: &str, peer: &AgentPeer) -> bool {
    let expected = format!("[{}]:{} ", peer.host, peer.port);
    let Some(key) = value.strip_prefix(&expected) else {
        return false;
    };
    let mut fields = key.split_whitespace();
    matches!(fields.next(), Some("ssh-ed25519"))
        && fields
            .next()
            .is_some_and(|encoded| !encoded.is_empty() && encoded.bytes().all(is_base64))
        && fields.next().is_none()
}

fn is_base64(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.len() != 17 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(format!(
            "agent lane id {:?} must be a 17-character base62 hash id",
            id
        ));
    }
    Ok(())
}

fn validate_peer(peer: &AgentPeer, id: &str) -> Result<(), String> {
    if !valid_uuid(&peer.node_id) || !valid_host(&peer.host) || peer.port < 1024 {
        return Err(format!("agent lane {:?} peer is invalid", id));
    }
    Ok(())
}

fn validate_listener(listener: &AgentListener, id: &str) -> Result<(), String> {
    if !matches!(listener.bind.as_str(), "127.0.0.1" | "::1") || listener.port < 1024 {
        return Err(format!(
            "agent lane {:?} listener must be unprivileged loopback",
            id
        ));
    }
    Ok(())
}

fn validate_restart(restart: &RestartPolicy, id: &str) -> Result<(), String> {
    if restart.initial_backoff_ms == 0
        || restart.max_backoff_ms < restart.initial_backoff_ms
        || restart.reset_after_secs == 0
    {
        return Err(format!("agent lane {:?} restart policy is invalid", id));
    }
    Ok(())
}

fn valid_host(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_ok()
        || value.len() <= 253
            && value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
}

fn default_enabled() -> bool {
    true
}
