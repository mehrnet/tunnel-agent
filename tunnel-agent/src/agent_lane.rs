use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

use crate::{
    agent_spec::{AgentDirection, AgentListener, AgentPeer, AgentTransport},
    agent_transports::{self, RatholeLane, WstunnelLane},
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
    pub hpnssh: Option<HpnSshLane>,
    pub rathole: Option<RatholeLane>,
    pub wstunnel: Option<WstunnelLane>,
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

pub(crate) type LaneCompilation = (Vec<ProcessSpec>, Vec<GeneratedFile>, Vec<ListenerClaim>);

impl AgentLane {
    pub(crate) fn seed_target(&self) -> Option<&AgentTarget> {
        match self.transport {
            AgentTransport::HpnSsh => self
                .hpnssh
                .as_ref()?
                .client
                .as_ref()
                .map(|client| &client.target),
            AgentTransport::Rathole => self.rathole.as_ref()?.target.as_ref(),
            AgentTransport::Wstunnel => self.wstunnel.as_ref()?.target.as_ref(),
        }
    }
}

pub fn validate(lane: &AgentLane, scope: NodeScope) -> Result<(), String> {
    validate_id(&lane.id)?;
    validate_peer(&lane.peer, &lane.id)?;
    validate_listener(&lane.listener, &lane.id)?;
    validate_restart(&lane.restart, &lane.id)?;
    if lane.direction != AgentDirection::ForeignToDomestic {
        return Err(format!(
            "agent lane {:?} must use foreign_to_domestic",
            lane.id
        ));
    }
    if !matches!(
        (scope, lane.role),
        (NodeScope::Domestic, AgentLaneRole::Server) | (NodeScope::Foreign, AgentLaneRole::Client)
    ) {
        return Err(format!(
            "agent lane {:?} role does not match node_scope",
            lane.id
        ));
    }
    match (lane.transport, &lane.hpnssh, &lane.rathole, &lane.wstunnel) {
        (AgentTransport::HpnSsh, Some(hpn), None, None) if valid_uuid(&hpn.cluster_id) => {
            match lane.role {
                AgentLaneRole::Server => validate_server(lane, hpn),
                AgentLaneRole::Client => validate_client(lane, hpn),
            }
        }
        (AgentTransport::Rathole, None, Some(rathole), None) => rathole.validate(lane),
        (AgentTransport::Wstunnel, None, None, Some(wstunnel)) => wstunnel.validate(lane),
        _ => Err(format!(
            "agent lane {:?} requires exactly its transport settings",
            lane.id
        )),
    }
}

pub fn compile(
    lane: &AgentLane,
    bundle_root: &Path,
    state_dir: &Path,
    check_only: bool,
) -> Result<LaneCompilation, String> {
    if let Some(hpn) = &lane.hpnssh {
        return Ok(match lane.role {
            AgentLaneRole::Server => compile_server(lane, bundle_root, state_dir, hpn),
            AgentLaneRole::Client => compile_client(lane, bundle_root, hpn),
        });
    }
    agent_transports::compile(lane, bundle_root, state_dir, check_only)
}

fn compile_server(
    lane: &AgentLane,
    bundle_root: &Path,
    state_dir: &Path,
    hpn: &HpnSshLane,
) -> LaneCompilation {
    let server = hpn.server.as_ref().expect("validated agent server");
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
        content: render_server_config(server, &lane.listener),
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
    (vec![process], vec![generated], listeners)
}

fn compile_client(lane: &AgentLane, bundle_root: &Path, hpn: &HpnSshLane) -> LaneCompilation {
    let client = hpn.client.as_ref().expect("validated agent client");
    let forward = format!(
        "{}:{}",
        authority(&lane.listener.bind, lane.listener.port),
        authority(&client.target.host, client.target.port)
    );
    let tunnel = ProcessSpec {
        id: format!("agent-lane-{}", lane.id),
        argv: vec![
            bundle_root.join("hpnssh/bin/hpnssh").display().to_string(),
            "-N".to_owned(),
            "-F".to_owned(),
            "/dev/null".to_owned(),
            "-o".to_owned(),
            "ExitOnForwardFailure=yes".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=20".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=3".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=yes".to_owned(),
            "-o".to_owned(),
            "GlobalKnownHostsFile=/dev/null".to_owned(),
            "-o".to_owned(),
            "IdentitiesOnly=yes".to_owned(),
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
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
    (vec![tunnel], Vec::new(), Vec::new())
}

pub(crate) fn compile_seed(
    target: &AgentTarget,
    bundle_root: &Path,
    state_dir: &Path,
) -> (ProcessSpec, ListenerClaim) {
    let family = if target.host == "::1" { "v6" } else { "v4" };
    let id = format!("agent-seed-{family}-{}", target.port);
    let seed = ProcessSpec {
        id: id.clone(),
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
            authority(&target.host, target.port),
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
        restart: RestartPolicy::default(),
    };
    let claim = ListenerClaim {
        owner: id,
        bind: target.host.clone(),
        port: target.port,
    };
    (seed, claim)
}

fn render_server_config(server: &HpnSshServer, listener: &AgentListener) -> String {
    format!(
        "Port {}\nListenAddress {}\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nPermitRootLogin prohibit-password\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nChallengeResponseAuthentication no\nAllowTcpForwarding remote\nPermitListen {}\nPermitOpen none\nPermitTunnel no\nAllowStreamLocalForwarding no\nGatewayPorts no\nX11Forwarding no\nAllowAgentForwarding no\nPermitTTY no\nUseDNS no\nClientAliveInterval 20\nClientAliveCountMax 3\nMaxSessions 0\n",
        server.port,
        server.bind,
        server.host_key_file,
        server.authorized_keys_file,
        authority(&listener.bind, listener.port),
    )
}

fn validate_server(lane: &AgentLane, hpn: &HpnSshLane) -> Result<(), String> {
    let Some(server) = &hpn.server else {
        return Err(format!(
            "agent lane {:?} server settings are required",
            lane.id
        ));
    };
    if hpn.client.is_some()
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

fn validate_client(lane: &AgentLane, hpn: &HpnSshLane) -> Result<(), String> {
    let Some(client) = &hpn.client else {
        return Err(format!(
            "agent lane {:?} client settings are required",
            lane.id
        ));
    };
    if hpn.server.is_some()
        || client.user.is_empty()
        || client.identity_file.is_empty()
        || client.known_hosts_file.is_empty()
        || !valid_known_host(&client.known_host, &lane.peer)
        || !client.target.is_loopback()
    {
        return Err(format!(
            "agent lane {:?} has invalid client settings",
            lane.id
        ));
    }
    Ok(())
}

impl AgentTarget {
    pub(crate) fn is_loopback(&self) -> bool {
        matches!(self.host.as_str(), "127.0.0.1" | "::1") && self.port >= 1024
    }
}

pub(crate) fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
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
