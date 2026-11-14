use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;

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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSpec {
    /// Usually `/opt/tunnel-agent/current`. The installer atomically advances
    /// this symlink only after a complete bundle has been verified.
    pub root: PathBuf,
}

const BUNDLED_ARTIFACTS: &[(&str, &str)] = &[
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
    fn permits(self, source: Self) -> bool {
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

#[derive(Clone, Debug, Deserialize)]
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

    fn artifact(&self) -> &str {
        match self {
            Self::HpnSshForward(spec) => &spec.artifact,
            Self::HpnSshServer(spec) => &spec.artifact,
            Self::WstunnelServer(spec) => &spec.artifact,
            Self::WstunnelReverse(spec) => &spec.artifact,
            Self::RatholeService(spec) => &spec.artifact,
            Self::TrustTunnelEndpoint(spec) => &spec.artifact,
        }
    }

    fn enabled(&self) -> bool {
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

pub fn load_config(path: &Path) -> Result<Config, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse_config(&raw)
}

pub fn parse_config(raw: &str) -> Result<Config, String> {
    let config: Config = serde_json::from_str(raw)
        .map_err(|error| format!("cannot parse configuration: {error}"))?;
    validate_config(&config)?;
    Ok(config)
}

pub fn write_config_atomic(path: &Path, raw: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("configuration path {} has no parent", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("next");
    fs::write(&temporary, raw)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))
}

pub fn validate_config(config: &Config) -> Result<(), String> {
    match config.version {
        1 => {
            if config.bundle.is_some() || !config.artifacts.is_empty() || !config.tunnels.is_empty()
            {
                return Err("manifest version 1 only supports processes".to_owned());
            }
            if config.processes.is_empty() {
                return Err("processes must not be empty".to_owned());
            }
        }
        2 => {
            if config.bundle.is_some() {
                return Err("manifest version 2 must not define bundle".to_owned());
            }
            if config.processes.is_empty() && config.tunnels.is_empty() {
                return Err("version 2 requires at least one process or tunnel".to_owned());
            }
        }
        3 => {
            if !config.artifacts.is_empty() {
                return Err(
                    "manifest version 3 uses the installed bundle and must not define artifacts"
                        .to_owned(),
                );
            }
            if !config.processes.is_empty() {
                return Err(
                    "manifest version 3 supports typed tunnels only; raw processes are not allowed"
                        .to_owned(),
                );
            }
            validate_bundle(
                config
                    .bundle
                    .as_ref()
                    .ok_or_else(|| "manifest version 3 requires bundle.root".to_owned())?,
            )?;
        }
        version => {
            return Err(format!(
                "unsupported manifest version {version}; expected 1, 2, or 3"
            ));
        }
    }

    validate_processes(&config.processes)?;
    validate_artifacts(&config.artifacts)?;

    let artifacts: HashSet<&str> = match &config.bundle {
        Some(_) => BUNDLED_ARTIFACTS.iter().map(|(id, _)| *id).collect(),
        None => config
            .artifacts
            .iter()
            .map(|artifact| artifact.id.as_str())
            .collect(),
    };
    let mut ids: HashSet<&str> = config
        .processes
        .iter()
        .map(|process| process.id.as_str())
        .collect();
    for tunnel in &config.tunnels {
        validate_id(tunnel.id(), "tunnel")?;
        if !ids.insert(tunnel.id()) {
            return Err(format!("duplicate process or tunnel id {:?}", tunnel.id()));
        }
        if tunnel.enabled() {
            validate_tunnel(tunnel, &artifacts)?;
        }
    }
    Ok(())
}

fn validate_bundle(bundle: &BundleSpec) -> Result<(), String> {
    if !bundle.root.is_absolute() {
        return Err("bundle.root must be an absolute path".to_owned());
    }
    if bundle.root.as_os_str().is_empty() {
        return Err("bundle.root must not be empty".to_owned());
    }
    Ok(())
}

fn validate_processes(processes: &[ProcessSpec]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for process in processes {
        validate_id(&process.id, "process")?;
        if !ids.insert(process.id.as_str()) {
            return Err(format!("duplicate process id {:?}", process.id));
        }
        if process.argv.is_empty() || process.argv[0].trim().is_empty() {
            return Err(format!(
                "process {:?} must contain a non-empty argv[0]",
                process.id
            ));
        }
        validate_restart(&process.id, &process.restart)?;
        if process
            .env
            .keys()
            .any(|key| key.is_empty() || key.contains('='))
        {
            return Err(format!(
                "process {:?} has an invalid environment key",
                process.id
            ));
        }
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[ArtifactSpec]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for artifact in artifacts {
        validate_id(&artifact.id, "artifact")?;
        if !ids.insert(artifact.id.as_str()) {
            return Err(format!("duplicate artifact id {:?}", artifact.id));
        }
        validate_safe_component(&artifact.version, "artifact version")?;
        validate_relative_path(&artifact.binary, "artifact binary")?;
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "artifact {:?} sha256 must be 64 hexadecimal characters",
                artifact.id
            ));
        }
        if let Some(binary_sha256) = &artifact.binary_sha256
            && (binary_sha256.len() != 64
                || !binary_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(format!(
                "artifact {:?} binary_sha256 must be 64 hexadecimal characters",
                artifact.id
            ));
        }
        if artifact.sources.is_empty() {
            return Err(format!(
                "artifact {:?} requires at least one source",
                artifact.id
            ));
        }
        for source in &artifact.sources {
            if !source.url.starts_with("https://") {
                return Err(format!("artifact {:?} source must use HTTPS", artifact.id));
            }
        }
    }
    Ok(())
}

fn validate_tunnel(tunnel: &TunnelSpec, artifacts: &HashSet<&str>) -> Result<(), String> {
    validate_id(tunnel.id(), "tunnel")?;
    if !artifacts.contains(tunnel.artifact()) {
        return Err(format!(
            "tunnel {:?} references unknown artifact {:?}",
            tunnel.id(),
            tunnel.artifact()
        ));
    }

    match tunnel {
        TunnelSpec::HpnSshForward(spec) => {
            validate_restart(&spec.id, &spec.restart)?;
            validate_ssh_peer(&spec.peer, &spec.id)?;
            validate_listener(&spec.listener, &spec.id)?;
            match spec.direction {
                HpnDirection::RemoteSocks | HpnDirection::LocalSocks
                    if spec.listener.target.is_some() =>
                {
                    return Err(format!(
                        "tunnel {:?} SOCKS modes cannot define listener.target",
                        spec.id
                    ));
                }
                HpnDirection::RemoteTcp if spec.listener.target.is_none() => {
                    return Err(format!(
                        "tunnel {:?} remote_tcp requires listener.target",
                        spec.id
                    ));
                }
                _ => {}
            }
        }
        TunnelSpec::HpnSshServer(spec) => {
            validate_restart(&spec.id, &spec.restart)?;
            if spec.config_file.as_os_str().is_empty() {
                return Err(format!(
                    "tunnel {:?} config_file must not be empty",
                    spec.id
                ));
            }
        }
        TunnelSpec::WstunnelServer(spec) => {
            validate_restart(&spec.id, &spec.restart)?;
            validate_listener(&spec.listener, &spec.id)?;
            if spec.listener.target.is_some() {
                return Err(format!(
                    "tunnel {:?} server listener cannot define target",
                    spec.id
                ));
            }
            validate_wstunnel_path(&spec.path_prefix, &spec.id)?;
            if spec.worker_threads == 0 {
                return Err(format!(
                    "tunnel {:?} worker_threads must be positive",
                    spec.id
                ));
            }
        }
        TunnelSpec::WstunnelReverse(spec) => {
            validate_restart(&spec.id, &spec.restart)?;
            validate_wstunnel_endpoint(&spec.endpoint, &spec.id)?;
            validate_tunnel_listener(&spec.remote_listener, &spec.id)?;
            if spec.worker_threads == 0 {
                return Err(format!(
                    "tunnel {:?} worker_threads must be positive",
                    spec.id
                ));
            }
            match spec.remote_listener.scheme {
                TunnelListenerScheme::Socks5 if spec.remote_listener.target.is_some() => {
                    return Err(format!(
                        "tunnel {:?} SOCKS listener cannot define target",
                        spec.id
                    ));
                }
                TunnelListenerScheme::Tcp if spec.remote_listener.target.is_none() => {
                    return Err(format!("tunnel {:?} TCP listener requires target", spec.id));
                }
                _ => {}
            }
        }
        TunnelSpec::RatholeService(spec) => validate_rathole(spec)?,
        TunnelSpec::TrustTunnelEndpoint(spec) => {
            validate_restart(&spec.id, &spec.restart)?;
            if spec.settings_file.as_os_str().is_empty()
                || spec.tls_hosts_file.as_os_str().is_empty()
            {
                return Err(format!(
                    "tunnel {:?} requires settings_file and tls_hosts_file",
                    spec.id
                ));
            }
            if spec.jobs == Some(0) {
                return Err(format!("tunnel {:?} jobs must be positive", spec.id));
            }
        }
    }
    Ok(())
}

fn validate_id(id: &str, subject: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "{subject} id {:?} must use only letters, numbers, '-' or '_'",
            id
        ));
    }
    Ok(())
}

fn validate_safe_component(value: &str, subject: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(format!(
            "{subject} {:?} is not a safe path component",
            value
        ));
    }
    Ok(())
}

fn validate_relative_path(path: &Path, subject: &str) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{subject} must be a non-empty relative path without traversal"
        ));
    }
    Ok(())
}

fn validate_restart(id: &str, restart: &RestartPolicy) -> Result<(), String> {
    if restart.initial_backoff_ms == 0 {
        return Err(format!(
            "process {id:?} initial_backoff_ms must be positive"
        ));
    }
    if restart.max_backoff_ms < restart.initial_backoff_ms {
        return Err(format!(
            "process {id:?} max_backoff_ms must be at least initial_backoff_ms"
        ));
    }
    if restart.reset_after_secs == 0 {
        return Err(format!("process {id:?} reset_after_secs must be positive"));
    }
    Ok(())
}

fn validate_ssh_peer(peer: &SshPeer, id: &str) -> Result<(), String> {
    if peer.host.trim().is_empty() || peer.user.trim().is_empty() || peer.port == 0 {
        return Err(format!(
            "tunnel {id:?} peer host, user, and port are required"
        ));
    }
    if peer.strict_host_key && peer.known_hosts_file.is_none() {
        return Err(format!(
            "tunnel {id:?} requires known_hosts_file when strict_host_key is true"
        ));
    }
    if peer
        .extra_options
        .iter()
        .any(|option| option.starts_with('-') || option.is_empty())
    {
        return Err(format!(
            "tunnel {id:?} extra_options must be bare SSH option values"
        ));
    }
    Ok(())
}

fn validate_listener(listener: &Listener, id: &str) -> Result<(), String> {
    if listener.bind.trim().is_empty() || listener.port == 0 {
        return Err(format!("tunnel {id:?} listener bind and port are required"));
    }
    if let Some(target) = &listener.target {
        validate_target(target, id)?;
    }
    Ok(())
}

fn validate_tunnel_listener(listener: &TunnelListener, id: &str) -> Result<(), String> {
    if listener.bind.trim().is_empty() || listener.port == 0 {
        return Err(format!(
            "tunnel {id:?} remote_listener bind and port are required"
        ));
    }
    if let Some(target) = &listener.target {
        validate_target(target, id)?;
    }
    Ok(())
}

fn validate_target(target: &TcpTarget, id: &str) -> Result<(), String> {
    if target.host.trim().is_empty() || target.port == 0 {
        return Err(format!("tunnel {id:?} target host and port are required"));
    }
    Ok(())
}

fn validate_wstunnel_path(path: &str, id: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("tunnel {id:?} path_prefix must start with '/'"));
    }
    Ok(())
}

fn validate_wstunnel_endpoint(endpoint: &WstunnelEndpoint, id: &str) -> Result<(), String> {
    if !endpoint.url.starts_with("wss://") {
        return Err(format!("tunnel {id:?} endpoint.url must use wss://"));
    }
    if endpoint.tls_sni.trim().is_empty() {
        return Err(format!("tunnel {id:?} endpoint.tls_sni must not be empty"));
    }
    validate_wstunnel_path(&endpoint.path_prefix, id)
}

fn validate_rathole(spec: &RatholeService) -> Result<(), String> {
    validate_restart(&spec.id, &spec.restart)?;
    if spec.services.is_empty() {
        return Err(format!(
            "tunnel {:?} requires at least one Rathole service",
            spec.id
        ));
    }
    if spec
        .transport
        .as_ref()
        .is_some_and(|transport| transport.kind != RatholeTransportKind::Tcp)
    {
        return Err(format!(
            "tunnel {:?} currently supports only Rathole TCP transport; TLS and Noise require their complete certificate/key contracts",
            spec.id
        ));
    }
    if spec.role == RatholeRole::Server && spec.transport.is_some() {
        return Err(format!(
            "tunnel {:?} does not support Rathole server transport tuning in this contract revision",
            spec.id
        ));
    }
    match spec.role {
        RatholeRole::Server
            if spec.control.bind.as_deref().unwrap_or_default().is_empty()
                || spec.control.remote.is_some() =>
        {
            return Err(format!(
                "tunnel {:?} Rathole server requires control.bind and rejects control.remote",
                spec.id
            ));
        }
        RatholeRole::Client
            if spec
                .control
                .remote
                .as_deref()
                .unwrap_or_default()
                .is_empty()
                || spec.control.bind.is_some() =>
        {
            return Err(format!(
                "tunnel {:?} Rathole client requires control.remote and rejects control.bind",
                spec.id
            ));
        }
        _ => {}
    }
    let mut ids = HashSet::new();
    for service in &spec.services {
        validate_id(&service.id, "Rathole service")?;
        if !ids.insert(service.id.as_str()) {
            return Err(format!(
                "tunnel {:?} has duplicate Rathole service {:?}",
                spec.id, service.id
            ));
        }
        if service.token.is_empty() {
            return Err(format!(
                "tunnel {:?} Rathole service {:?} token must not be empty",
                spec.id, service.id
            ));
        }
        match spec.role {
            RatholeRole::Server
                if service.bind.as_deref().unwrap_or_default().is_empty()
                    || service.local.is_some() =>
            {
                return Err(format!(
                    "tunnel {:?} server service {:?} requires bind and rejects local",
                    spec.id, service.id
                ));
            }
            RatholeRole::Client
                if service.local.as_deref().unwrap_or_default().is_empty()
                    || service.bind.is_some() =>
            {
                return Err(format!(
                    "tunnel {:?} client service {:?} requires local and rejects bind",
                    spec.id, service.id
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn compile(config: &Config, state_dir: &Path) -> Result<CompiledPlan, String> {
    validate_config(config)?;
    let artifact_paths = resolve_artifact_paths(config, state_dir)?;
    let mut processes = config.processes.clone();
    let mut generated_files = Vec::new();
    let mut listeners = Vec::new();

    for tunnel in &config.tunnels {
        if !tunnel.enabled() {
            continue;
        }
        let binary = artifact_paths
            .get(tunnel.artifact())
            .ok_or_else(|| format!("tunnel {:?} references unavailable artifact", tunnel.id()))?;
        match tunnel {
            TunnelSpec::HpnSshForward(spec) => {
                processes.push(compile_hpn_forward(spec, binary));
                if spec.direction == HpnDirection::LocalSocks {
                    listeners.push(ListenerClaim {
                        owner: spec.id.clone(),
                        bind: spec.listener.bind.clone(),
                        port: spec.listener.port,
                    });
                }
            }
            TunnelSpec::HpnSshServer(spec) => {
                processes.push(ProcessSpec {
                    id: spec.id.clone(),
                    argv: vec![
                        binary.display().to_string(),
                        "-D".to_owned(),
                        "-f".to_owned(),
                        spec.config_file.display().to_string(),
                    ],
                    env: BTreeMap::new(),
                    workdir: None,
                    restart: spec.restart.clone(),
                });
            }
            TunnelSpec::WstunnelServer(spec) => {
                processes.push(compile_wstunnel_server(spec, binary));
                listeners.push(ListenerClaim {
                    owner: spec.id.clone(),
                    bind: spec.listener.bind.clone(),
                    port: spec.listener.port,
                });
            }
            TunnelSpec::WstunnelReverse(spec) => {
                processes.push(compile_wstunnel_reverse(spec, binary))
            }
            TunnelSpec::RatholeService(spec) => {
                let path = state_dir
                    .join("generated")
                    .join(format!("{}.toml", spec.id));
                generated_files.push(GeneratedFile {
                    path: path.clone(),
                    content: render_rathole(spec),
                });
                processes.push(ProcessSpec {
                    id: spec.id.clone(),
                    argv: vec![
                        binary.display().to_string(),
                        format!("--{}", rathole_role_name(spec.role)),
                        path.display().to_string(),
                    ],
                    env: BTreeMap::new(),
                    workdir: None,
                    restart: spec.restart.clone(),
                });
                if spec.role == RatholeRole::Server {
                    let control = spec
                        .control
                        .bind
                        .as_ref()
                        .expect("validated rathole server control bind");
                    listeners.push(listener_claim(&spec.id, control)?);
                    for service in &spec.services {
                        listeners.push(listener_claim(
                            &spec.id,
                            service
                                .bind
                                .as_ref()
                                .expect("validated rathole server service bind"),
                        )?);
                    }
                }
            }
            TunnelSpec::TrustTunnelEndpoint(spec) => {
                let mut argv = vec![
                    binary.display().to_string(),
                    spec.settings_file.display().to_string(),
                    spec.tls_hosts_file.display().to_string(),
                ];
                if let Some(level) = spec.log_level {
                    argv.extend([
                        "--loglvl".to_owned(),
                        trust_log_level_name(level).to_owned(),
                    ]);
                }
                if let Some(jobs) = spec.jobs {
                    argv.extend(["--jobs".to_owned(), jobs.to_string()]);
                }
                processes.push(ProcessSpec {
                    id: spec.id.clone(),
                    argv,
                    env: BTreeMap::new(),
                    workdir: None,
                    restart: spec.restart.clone(),
                });
            }
        }
    }

    validate_processes(&processes)?;
    validate_listener_conflicts(&listeners)?;
    Ok(CompiledPlan {
        processes,
        generated_files,
        listeners,
    })
}

fn resolve_artifact_paths(
    config: &Config,
    state_dir: &Path,
) -> Result<HashMap<String, PathBuf>, String> {
    if let Some(bundle) = &config.bundle {
        return BUNDLED_ARTIFACTS
            .iter()
            .map(|(id, relative)| Ok(((*id).to_owned(), bundle.root.join(relative))))
            .collect();
    }
    Ok(config
        .artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.id.clone(),
                artifact_binary_path(state_dir, artifact),
            )
        })
        .collect())
}

fn compile_hpn_forward(spec: &HpnSshForward, binary: &Path) -> ProcessSpec {
    let mut argv = vec![
        binary.display().to_string(),
        "-N".to_owned(),
        "-o".to_owned(),
        "ExitOnForwardFailure=yes".to_owned(),
        "-o".to_owned(),
        "ServerAliveInterval=20".to_owned(),
        "-o".to_owned(),
        "ServerAliveCountMax=3".to_owned(),
        "-p".to_owned(),
        spec.peer.port.to_string(),
    ];
    if spec.peer.strict_host_key {
        argv.extend(["-o".to_owned(), "StrictHostKeyChecking=yes".to_owned()]);
        argv.extend([
            "-o".to_owned(),
            format!(
                "UserKnownHostsFile={}",
                spec.peer
                    .known_hosts_file
                    .as_ref()
                    .expect("validated known_hosts")
                    .display()
            ),
        ]);
    } else {
        argv.extend(["-o".to_owned(), "StrictHostKeyChecking=no".to_owned()]);
    }
    if let Some(identity) = &spec.peer.identity_file {
        argv.extend(["-i".to_owned(), identity.display().to_string()]);
    }
    for option in &spec.peer.extra_options {
        argv.extend(["-o".to_owned(), option.clone()]);
    }
    match spec.direction {
        HpnDirection::RemoteSocks => argv.extend([
            "-R".to_owned(),
            format!("{}:{}", spec.listener.bind, spec.listener.port),
        ]),
        HpnDirection::RemoteTcp => {
            let target = spec
                .listener
                .target
                .as_ref()
                .expect("validated remote TCP target");
            argv.extend([
                "-R".to_owned(),
                format!(
                    "{}:{}:{}:{}",
                    spec.listener.bind, spec.listener.port, target.host, target.port
                ),
            ]);
        }
        HpnDirection::LocalSocks => argv.extend([
            "-D".to_owned(),
            format!("{}:{}", spec.listener.bind, spec.listener.port),
        ]),
    }
    argv.push(format!("{}@{}", spec.peer.user, spec.peer.host));
    ProcessSpec {
        id: spec.id.clone(),
        argv,
        env: BTreeMap::new(),
        workdir: None,
        restart: spec.restart.clone(),
    }
}

fn compile_wstunnel_server(spec: &WstunnelServer, binary: &Path) -> ProcessSpec {
    ProcessSpec {
        id: spec.id.clone(),
        argv: vec![
            binary.display().to_string(),
            "server".to_owned(),
            "--log-lvl".to_owned(),
            "WARN".to_owned(),
            "--nb-worker-threads".to_owned(),
            spec.worker_threads.to_string(),
            "--restrict-http-upgrade-path-prefix".to_owned(),
            spec.path_prefix.clone(),
            "--tls-certificate".to_owned(),
            spec.tls_certificate.display().to_string(),
            "--tls-private-key".to_owned(),
            spec.tls_private_key.display().to_string(),
            format!("wss://{}:{}", spec.listener.bind, spec.listener.port),
        ],
        env: BTreeMap::new(),
        workdir: None,
        restart: spec.restart.clone(),
    }
}

fn compile_wstunnel_reverse(spec: &WstunnelReverse, binary: &Path) -> ProcessSpec {
    let listener = match spec.remote_listener.scheme {
        TunnelListenerScheme::Socks5 => format!(
            "socks5://{}:{}",
            spec.remote_listener.bind, spec.remote_listener.port
        ),
        TunnelListenerScheme::Tcp => {
            let target = spec
                .remote_listener
                .target
                .as_ref()
                .expect("validated TCP target");
            format!(
                "tcp://{}:{}:{}:{}",
                spec.remote_listener.bind, spec.remote_listener.port, target.host, target.port
            )
        }
    };
    let mut argv = vec![
        binary.display().to_string(),
        "client".to_owned(),
        "--log-lvl".to_owned(),
        "WARN".to_owned(),
        "--nb-worker-threads".to_owned(),
        spec.worker_threads.to_string(),
        "--tls-sni-override".to_owned(),
        spec.endpoint.tls_sni.clone(),
    ];
    if spec.endpoint.verify_certificate {
        argv.push("--tls-verify-certificate".to_owned());
    }
    argv.extend([
        "--http-upgrade-path-prefix".to_owned(),
        spec.endpoint.path_prefix.clone(),
        "--reverse-tunnel-connection-retry-max-backoff".to_owned(),
        "15s".to_owned(),
        "--remote-to-local".to_owned(),
        listener,
        spec.endpoint.url.clone(),
    ]);
    ProcessSpec {
        id: spec.id.clone(),
        argv,
        env: BTreeMap::new(),
        workdir: None,
        restart: spec.restart.clone(),
    }
}

fn render_rathole(spec: &RatholeService) -> String {
    let mut output = String::new();
    match spec.role {
        RatholeRole::Server => {
            output.push_str("[server]\n");
            output.push_str(&format!(
                "bind_addr = {}\n",
                toml_string(spec.control.bind.as_ref().expect("validated server bind"))
            ));
        }
        RatholeRole::Client => {
            output.push_str("[client]\n");
            output.push_str(&format!(
                "remote_addr = {}\n",
                toml_string(
                    spec.control
                        .remote
                        .as_ref()
                        .expect("validated client remote")
                )
            ));
        }
    }
    if let Some(transport) = &spec.transport {
        // Rathole documents TCP knobs under client.transport.tcp. Server-side
        // transport tuning has a different surface and is intentionally rejected
        // until it has its own complete contract.
        output.push_str("\n[client.transport]\n");
        output.push_str(&format!(
            "type = {}\n",
            toml_string(rathole_transport_name(transport.kind))
        ));
        if transport.nodelay.is_some() || transport.keepalive_secs.is_some() {
            output.push_str("\n[client.transport.tcp]\n");
            if let Some(nodelay) = transport.nodelay {
                output.push_str(&format!("nodelay = {nodelay}\n"));
            }
            if let Some(keepalive_secs) = transport.keepalive_secs {
                output.push_str(&format!("keepalive_secs = {keepalive_secs}\n"));
            }
        }
    }
    for service in &spec.services {
        let header = match spec.role {
            RatholeRole::Server => format!("\n[server.services.{}]\n", service.id),
            RatholeRole::Client => format!("\n[client.services.{}]\n", service.id),
        };
        output.push_str(&header);
        output.push_str(&format!("token = {}\n", toml_string(&service.token)));
        match spec.role {
            RatholeRole::Server => output.push_str(&format!(
                "bind_addr = {}\n",
                toml_string(
                    service
                        .bind
                        .as_ref()
                        .expect("validated server service bind")
                )
            )),
            RatholeRole::Client => output.push_str(&format!(
                "local_addr = {}\n",
                toml_string(
                    service
                        .local
                        .as_ref()
                        .expect("validated client service local")
                )
            )),
        }
    }
    output
}

fn toml_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn rathole_role_name(role: RatholeRole) -> &'static str {
    match role {
        RatholeRole::Server => "server",
        RatholeRole::Client => "client",
    }
}
fn rathole_transport_name(kind: RatholeTransportKind) -> &'static str {
    match kind {
        RatholeTransportKind::Tcp => "tcp",
        RatholeTransportKind::Tls => "tls",
        RatholeTransportKind::Noise => "noise",
    }
}
fn trust_log_level_name(level: TrustTunnelLogLevel) -> &'static str {
    match level {
        TrustTunnelLogLevel::Info => "info",
        TrustTunnelLogLevel::Debug => "debug",
        TrustTunnelLogLevel::Trace => "trace",
    }
}

fn listener_claim(owner: &str, address: &str) -> Result<ListenerClaim, String> {
    let parsed: SocketAddr = address.parse().map_err(|_| {
        format!("tunnel {owner:?} address {address:?} must be an IP:port socket address")
    })?;
    Ok(ListenerClaim {
        owner: owner.to_owned(),
        bind: parsed.ip().to_string(),
        port: parsed.port(),
    })
}

fn validate_listener_conflicts(listeners: &[ListenerClaim]) -> Result<(), String> {
    let mut claimed = HashMap::new();
    for listener in listeners {
        let key = (&listener.bind, listener.port);
        if let Some(previous) = claimed.insert(key, &listener.owner) {
            return Err(format!(
                "local listener conflict: {previous:?} and {:?} both claim {}:{}",
                listener.owner, listener.bind, listener.port
            ));
        }
    }
    Ok(())
}

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
        .map_err(|error| format!("cannot publish download: {error}"))?;
    Ok(())
}

fn install_download(
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

fn sha256_file(path: &Path) -> Result<String, String> {
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

pub fn write_generated_files(plan: &CompiledPlan) -> Result<(), String> {
    for generated in &plan.generated_files {
        let parent = generated.path.parent().expect("generated file has parent");
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        let temporary = generated.path.with_extension("tmp");
        fs::write(&temporary, &generated.content)
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        fs::rename(&temporary, &generated.path)
            .map_err(|error| format!("cannot publish {}: {error}", generated.path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn artifact(id: &str) -> ArtifactSpec {
        ArtifactSpec {
            id: id.to_owned(),
            version: "v1".to_owned(),
            binary: PathBuf::from(id),
            sha256: "0".repeat(64),
            binary_sha256: None,
            package: PackageFormat::Binary,
            sources: vec![ArtifactSource {
                url: "https://example.test/artifact".to_owned(),
                scope: NodeScope::Any,
                priority: 0,
            }],
        }
    }

    fn process(id: &str) -> ProcessSpec {
        ProcessSpec {
            id: id.to_owned(),
            argv: vec!["/bin/true".to_owned()],
            env: BTreeMap::new(),
            workdir: None,
            restart: RestartPolicy::default(),
        }
    }

    #[test]
    fn accepts_a_legacy_manifest() {
        let config = Config {
            version: 1,
            node_scope: NodeScope::Any,
            bundle: None,
            artifacts: vec![],
            processes: vec![process("first")],
            tunnels: vec![],
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn rejects_duplicate_process_ids() {
        let config = Config {
            version: 1,
            node_scope: NodeScope::Any,
            bundle: None,
            artifacts: vec![],
            processes: vec![process("same"), process("same")],
            tunnels: vec![],
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn compiles_a_rathole_server_contract() {
        let config = Config {
            version: 2,
            node_scope: NodeScope::Foreign,
            bundle: None,
            processes: vec![],
            artifacts: vec![artifact("rathole")],
            tunnels: vec![TunnelSpec::RatholeService(RatholeService {
                id: "rathole_de".to_owned(),
                enabled: true,
                artifact: "rathole".to_owned(),
                role: RatholeRole::Server,
                control: RatholeControl {
                    bind: Some("0.0.0.0:2333".to_owned()),
                    remote: None,
                },
                transport: None,
                services: vec![RatholeServiceMapping {
                    id: "web".to_owned(),
                    token: "secret".to_owned(),
                    bind: Some("0.0.0.0:8443".to_owned()),
                    local: None,
                }],
                restart: RestartPolicy::default(),
            })],
        };
        let plan =
            compile(&config, Path::new("/var/lib/tunnel-agent")).expect("valid rathole plan");
        assert_eq!(plan.processes[0].argv[1], "--server");
        assert!(
            plan.generated_files[0]
                .content
                .contains("[server.services.web]")
        );
        assert_eq!(plan.listeners.len(), 2);
    }

    #[test]
    fn rejects_listener_collisions_between_contracts() {
        let mut first = artifact("wstunnel");
        first.binary = PathBuf::from("wstunnel");
        let server = |id: &str| {
            TunnelSpec::WstunnelServer(WstunnelServer {
                id: id.to_owned(),
                enabled: true,
                artifact: "wstunnel".to_owned(),
                listener: Listener {
                    bind: "0.0.0.0".to_owned(),
                    port: 4444,
                    target: None,
                },
                tls_certificate: PathBuf::from("cert.pem"),
                tls_private_key: PathBuf::from("key.pem"),
                path_prefix: "/tunnel".to_owned(),
                worker_threads: 1,
                restart: RestartPolicy::default(),
            })
        };
        let config = Config {
            version: 2,
            node_scope: NodeScope::Domestic,
            bundle: None,
            artifacts: vec![first],
            processes: vec![],
            tunnels: vec![server("one"), server("two")],
        };
        assert!(compile(&config, Path::new("/state")).is_err());
    }

    #[test]
    fn rejects_a_socks_target() {
        let spec = HpnSshForward {
            id: "bad".to_owned(),
            enabled: true,
            artifact: "hpn".to_owned(),
            direction: HpnDirection::RemoteSocks,
            peer: SshPeer {
                host: "peer".to_owned(),
                user: "root".to_owned(),
                port: 2222,
                identity_file: None,
                known_hosts_file: Some(PathBuf::from("known_hosts")),
                strict_host_key: true,
                extra_options: vec![],
            },
            listener: Listener {
                bind: "127.0.0.1".to_owned(),
                port: 10810,
                target: Some(TcpTarget {
                    host: "127.0.0.1".to_owned(),
                    port: 8443,
                }),
            },
            restart: RestartPolicy::default(),
        };
        let config = Config {
            version: 2,
            node_scope: NodeScope::Foreign,
            bundle: None,
            artifacts: vec![artifact("hpn")],
            processes: vec![],
            tunnels: vec![TunnelSpec::HpnSshForward(spec)],
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn disabled_tunnel_does_not_require_its_artifact() {
        let raw = r#"{
          "version": 2,
          "tunnels": [{
            "contract": "wstunnel.server.v1",
            "id": "retired_wstunnel",
            "enabled": false,
            "artifact": "removed_wstunnel",
            "listener": {"bind": "0.0.0.0", "port": 4444},
            "tls_certificate": "/unused/cert.pem",
            "tls_private_key": "/unused/key.pem",
            "path_prefix": "/unused"
          }]
        }"#;
        let config: Config = serde_json::from_str(raw).expect("valid disabled tunnel schema");
        let plan = compile(&config, Path::new("/state")).expect("disabled tunnel is ignored");
        assert!(plan.processes.is_empty());
        assert!(plan.listeners.is_empty());
    }

    #[test]
    fn bundled_manifest_resolves_preinstalled_transport_binaries() {
        let raw = r#"{
          "version": 3,
          "bundle": {"root": "/opt/tunnel-agent/current"},
          "tunnels": [{
            "contract": "wstunnel.server.v1",
            "id": "canary",
            "artifact": "wstunnel",
            "listener": {"bind": "127.0.0.1", "port": 14444},
            "tls_certificate": "/etc/tunnel-agent/cert.pem",
            "tls_private_key": "/etc/tunnel-agent/key.pem",
            "path_prefix": "/canary"
          }]
        }"#;
        let config: Config = serde_json::from_str(raw).expect("valid bundled manifest");
        let plan = compile(&config, Path::new("/unused-state")).expect("bundled plan");
        assert_eq!(
            plan.processes[0].argv[0],
            "/opt/tunnel-agent/current/bin/wstunnel"
        );
        assert!(sync_artifacts(&config, Path::new("/unused-state"), None).is_err());
    }

    #[test]
    fn compiles_each_transport_contract() {
        let raw = r#"{
          "version": 2,
          "artifacts": [
            {"id":"hpn_client","version":"v1","binary":"hpnssh","sha256":"0000000000000000000000000000000000000000000000000000000000000000","sources":[{"url":"https://example.test/hpn-client"}]},
            {"id":"hpn_server","version":"v1","binary":"hpnsshd","sha256":"0000000000000000000000000000000000000000000000000000000000000000","sources":[{"url":"https://example.test/hpn-server"}]},
            {"id":"wstunnel","version":"v1","binary":"wstunnel","sha256":"0000000000000000000000000000000000000000000000000000000000000000","sources":[{"url":"https://example.test/wstunnel"}]},
            {"id":"rathole","version":"v1","binary":"rathole","sha256":"0000000000000000000000000000000000000000000000000000000000000000","sources":[{"url":"https://example.test/rathole"}]},
            {"id":"trusttunnel","version":"v1","binary":"trusttunnel_endpoint","sha256":"0000000000000000000000000000000000000000000000000000000000000000","sources":[{"url":"https://example.test/trusttunnel"}]}
          ],
          "tunnels": [
            {"contract":"hpnssh.forward.v1","id":"hpn_forward","artifact":"hpn_client","direction":"remote_tcp","peer":{"host":"peer.example","user":"tunnel","port":2222,"known_hosts_file":"/etc/tunnel-agent/known_hosts"},"listener":{"bind":"127.0.0.1","port":10810,"target":{"host":"127.0.0.1","port":48443}}},
            {"contract":"hpnssh.server.v1","id":"hpn_server","artifact":"hpn_server","config_file":"/etc/hpnssh/sshd_config"},
            {"contract":"wstunnel.server.v1","id":"wstunnel_server","artifact":"wstunnel","listener":{"bind":"0.0.0.0","port":4444},"tls_certificate":"/etc/tunnel-agent/cert.pem","tls_private_key":"/etc/tunnel-agent/key.pem","path_prefix":"/tunnel"},
            {"contract":"wstunnel.reverse.v1","id":"wstunnel_reverse","artifact":"wstunnel","endpoint":{"url":"wss://peer.example:4444","tls_sni":"peer.example","path_prefix":"/tunnel"},"remote_listener":{"scheme":"socks5","bind":"127.0.0.1","port":10910}},
            {"contract":"rathole.service.v1","id":"rathole_server","artifact":"rathole","role":"server","control":{"bind":"0.0.0.0:2333"},"services":[{"id":"xhttp","token":"secret","bind":"0.0.0.0:8443"}]},
            {"contract":"trusttunnel.endpoint.v1","id":"trusttunnel_endpoint","artifact":"trusttunnel","settings_file":"/etc/trusttunnel/vpn.toml","tls_hosts_file":"/etc/trusttunnel/hosts.toml"}
          ]
        }"#;
        let config: Config = serde_json::from_str(raw).expect("valid typed manifest");
        let plan = compile(&config, Path::new("/state")).expect("all contracts compile");
        assert_eq!(plan.processes.len(), 6);
        assert_eq!(plan.listeners.len(), 3);
        assert!(
            plan.generated_files[0]
                .content
                .contains("[server.services.xhttp]")
        );
    }

    #[test]
    fn binary_package_is_verified_before_installation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tunnel-agent-test-{unique}"));
        fs::create_dir_all(&root).expect("create test directory");
        let source = root.join("source");
        let destination = root.join("destination");
        fs::write(&source, b"test-binary").expect("write source");
        let mut package = artifact("test");
        package.sha256 = sha256_file(&source).expect("hash source");
        install_download(&package, &source, &destination).expect("verified installation");
        assert_eq!(
            fs::read(&destination).expect("read destination"),
            b"test-binary"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rathole_client_tcp_tuning_uses_the_documented_table() {
        let raw = r#"{
          "version": 2,
          "artifacts": [{
            "id": "rathole", "version": "v1", "binary": "rathole",
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
            "sources": [{"url": "https://example.test/rathole"}]
          }],
          "tunnels": [{
            "contract": "rathole.service.v1", "id": "client", "artifact": "rathole",
            "role": "client", "control": {"remote": "server.example:2333"},
            "transport": {"type": "tcp", "nodelay": true, "keepalive_secs": 20},
            "services": [{"id": "xhttp", "token": "secret", "local": "127.0.0.1:48443"}]
          }]
        }"#;
        let config: Config = serde_json::from_str(raw).expect("valid rathole client schema");
        let plan = compile(&config, Path::new("/state")).expect("valid rathole client plan");
        assert!(
            plan.generated_files[0]
                .content
                .contains("[client.transport.tcp]")
        );
        assert!(plan.generated_files[0].content.contains("nodelay = true"));
    }
}
