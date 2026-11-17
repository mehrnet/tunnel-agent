use std::{collections::BTreeMap, path::PathBuf};

use serde::Deserialize;

use crate::agent_spec::{AgentLanePlan, AgentSpec};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub node_scope: NodeScope,
    /// Version 3 manifests execute only the immutable transport bundle installed
    /// by the bootstrapper. They never fetch a tunnel binary at lane-apply time.
    #[serde(default)]
    pub bundle: Option<BundleSpec>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactSpec>,
    #[serde(default)]
    pub processes: Vec<ProcessSpec>,
    #[serde(default)]
    pub tunnels: Vec<TunnelSpec>,
    #[serde(default)]
    pub agent: AgentSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSpec {
    /// Usually `/opt/tunnel-agent/current`. The installer atomically advances
    /// this symlink only after a complete bundle has been verified.
    pub root: PathBuf,
}

pub(super) const BUNDLED_ARTIFACTS: &[(&str, &str)] = &[
    ("hpnssh", "hpnssh/bin/hpnssh"),
    ("hpnsshd", "hpnssh/sbin/hpnsshd"),
    ("wstunnel", "bin/wstunnel"),
    ("rathole", "bin/rathole"),
    ("trusttunnel", "bin/trusttunnel_endpoint"),
];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeScope {
    #[default]
    Any,
    Domestic,
    Foreign,
}

impl NodeScope {
    pub(super) fn permits(self, source: Self) -> bool {
        source == Self::Any || self == Self::Any || self == source
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSpec {
    pub id: String,
    pub version: String,
    pub binary: PathBuf,
    /// SHA-256 of the downloaded package. This pins every mirror to identical bytes.
    pub sha256: String,
    /// Optional SHA-256 of the extracted binary for multi-file archive packages.
    pub binary_sha256: Option<String>,
    pub sources: Vec<ArtifactSource>,
    #[serde(default)]
    pub package: PackageFormat,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSource {
    pub url: String,
    #[serde(default)]
    pub scope: NodeScope,
    #[serde(default)]
    pub priority: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageFormat {
    #[default]
    Binary,
    TarGz,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub workdir: Option<PathBuf>,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RestartPolicy {
    #[serde(default = "default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,
    #[serde(default = "default_reset_after_secs")]
    pub reset_after_secs: u64,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: default_initial_backoff_ms(),
            max_backoff_ms: default_max_backoff_ms(),
            reset_after_secs: default_reset_after_secs(),
        }
    }
}

fn default_initial_backoff_ms() -> u64 {
    1_000
}

fn default_max_backoff_ms() -> u64 {
    60_000
}

fn default_reset_after_secs() -> u64 {
    60
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "contract")]
pub enum TunnelSpec {
    #[serde(rename = "hpnssh.forward.v1")]
    HpnSshForward(HpnSshForward),
    #[serde(rename = "hpnssh.server.v1")]
    HpnSshServer(HpnSshServer),
    #[serde(rename = "wstunnel.server.v1")]
    WstunnelServer(WstunnelServer),
    #[serde(rename = "wstunnel.reverse.v1")]
    WstunnelReverse(WstunnelReverse),
    #[serde(rename = "rathole.service.v1")]
    RatholeService(RatholeService),
    #[serde(rename = "trusttunnel.endpoint.v1")]
    TrustTunnelEndpoint(TrustTunnelEndpoint),
}

impl TunnelSpec {
    pub fn id(&self) -> &str {
        match self {
            Self::HpnSshForward(spec) => &spec.id,
            Self::HpnSshServer(spec) => &spec.id,
            Self::WstunnelServer(spec) => &spec.id,
            Self::WstunnelReverse(spec) => &spec.id,
            Self::RatholeService(spec) => &spec.id,
            Self::TrustTunnelEndpoint(spec) => &spec.id,
        }
    }

    pub(super) fn artifact(&self) -> &str {
        match self {
            Self::HpnSshForward(spec) => &spec.artifact,
            Self::HpnSshServer(spec) => &spec.artifact,
            Self::WstunnelServer(spec) => &spec.artifact,
            Self::WstunnelReverse(spec) => &spec.artifact,
            Self::RatholeService(spec) => &spec.artifact,
            Self::TrustTunnelEndpoint(spec) => &spec.artifact,
        }
    }

    pub(super) fn enabled(&self) -> bool {
        match self {
            Self::HpnSshForward(spec) => spec.enabled,
            Self::HpnSshServer(spec) => spec.enabled,
            Self::WstunnelServer(spec) => spec.enabled,
            Self::WstunnelReverse(spec) => spec.enabled,
            Self::RatholeService(spec) => spec.enabled,
            Self::TrustTunnelEndpoint(spec) => spec.enabled,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HpnSshForward {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub direction: HpnDirection,
    pub peer: SshPeer,
    pub listener: Listener,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HpnDirection {
    RemoteSocks,
    RemoteTcp,
    LocalSocks,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshPeer {
    pub host: String,
    pub user: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub identity_file: Option<PathBuf>,
    pub known_hosts_file: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub strict_host_key: bool,
    #[serde(default)]
    pub extra_options: Vec<String>,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub bind: String,
    pub port: u16,
    pub target: Option<TcpTarget>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HpnSshServer {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub config_file: PathBuf,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WstunnelServer {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub listener: Listener,
    pub tls_certificate: PathBuf,
    pub tls_private_key: PathBuf,
    pub path_prefix: String,
    #[serde(default = "default_wstunnel_worker_threads")]
    pub worker_threads: u16,
    #[serde(default)]
    pub restart: RestartPolicy,
}

fn default_wstunnel_worker_threads() -> u16 {
    1
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WstunnelReverse {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub endpoint: WstunnelEndpoint,
    pub remote_listener: TunnelListener,
    #[serde(default = "default_wstunnel_worker_threads")]
    pub worker_threads: u16,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WstunnelEndpoint {
    pub url: String,
    pub tls_sni: String,
    pub path_prefix: String,
    #[serde(default = "default_true")]
    pub verify_certificate: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelListener {
    pub scheme: TunnelListenerScheme,
    pub bind: String,
    pub port: u16,
    pub target: Option<TcpTarget>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelListenerScheme {
    Socks5,
    Tcp,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatholeService {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub role: RatholeRole,
    pub control: RatholeControl,
    pub services: Vec<RatholeServiceMapping>,
    pub transport: Option<RatholeTransport>,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RatholeRole {
    Server,
    Client,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatholeControl {
    pub bind: Option<String>,
    pub remote: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatholeServiceMapping {
    pub id: String,
    pub token: String,
    pub bind: Option<String>,
    pub local: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatholeTransport {
    #[serde(rename = "type")]
    pub kind: RatholeTransportKind,
    #[serde(default)]
    pub nodelay: Option<bool>,
    #[serde(default)]
    pub keepalive_secs: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RatholeTransportKind {
    Tcp,
    Tls,
    Noise,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustTunnelEndpoint {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub artifact: String,
    pub settings_file: PathBuf,
    pub tls_hosts_file: PathBuf,
    pub jobs: Option<u16>,
    pub log_level: Option<TrustTunnelLogLevel>,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTunnelLogLevel {
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Debug)]
pub struct CompiledPlan {
    pub processes: Vec<ProcessSpec>,
    pub generated_files: Vec<GeneratedFile>,
    pub listeners: Vec<ListenerClaim>,
    pub agent_lanes: Vec<AgentLanePlan>,
}

#[derive(Clone, Debug)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ListenerClaim {
    pub owner: String,
    pub bind: String,
    pub port: u16,
}
