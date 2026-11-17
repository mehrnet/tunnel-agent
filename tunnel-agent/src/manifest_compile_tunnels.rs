use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use super::{
    GeneratedFile, HpnDirection, HpnSshForward, HpnSshServer, ListenerClaim, ProcessSpec,
    RatholeRole, RatholeService, RatholeTransportKind, TrustTunnelEndpoint, TrustTunnelLogLevel,
    TunnelListenerScheme, TunnelSpec, WstunnelReverse, WstunnelServer,
};

pub(super) struct TunnelCompilation {
    pub(super) processes: Vec<ProcessSpec>,
    pub(super) generated_files: Vec<GeneratedFile>,
    pub(super) listeners: Vec<ListenerClaim>,
}

pub(super) fn compile_tunnels(
    config: &super::Config,
    state_dir: &Path,
    artifact_paths: &std::collections::HashMap<String, PathBuf>,
) -> Result<TunnelCompilation, String> {
    let mut compilation = TunnelCompilation {
        processes: config.processes.clone(),
        generated_files: Vec::new(),
        listeners: Vec::new(),
    };
    for tunnel in &config.tunnels {
        if !tunnel.enabled() {
            continue;
        }
        let binary = artifact_paths
            .get(tunnel.artifact())
            .ok_or_else(|| format!("tunnel {:?} references unavailable artifact", tunnel.id()))?;
        compile_tunnel(tunnel, binary, state_dir, &mut compilation)?;
    }
    Ok(compilation)
}

fn compile_tunnel(
    tunnel: &TunnelSpec,
    binary: &Path,
    state_dir: &Path,
    compilation: &mut TunnelCompilation,
) -> Result<(), String> {
    match tunnel {
        TunnelSpec::HpnSshForward(spec) => {
            compilation
                .processes
                .push(compile_hpn_forward(spec, binary));
            if spec.direction == HpnDirection::LocalSocks {
                compilation.listeners.push(ListenerClaim {
                    owner: spec.id.clone(),
                    bind: spec.listener.bind.clone(),
                    port: spec.listener.port,
                });
            }
        }
        TunnelSpec::HpnSshServer(spec) => {
            compilation.processes.push(compile_hpn_server(spec, binary));
        }
        TunnelSpec::WstunnelServer(spec) => {
            compilation
                .processes
                .push(compile_wstunnel_server(spec, binary));
            compilation.listeners.push(ListenerClaim {
                owner: spec.id.clone(),
                bind: spec.listener.bind.clone(),
                port: spec.listener.port,
            });
        }
        TunnelSpec::WstunnelReverse(spec) => {
            compilation
                .processes
                .push(compile_wstunnel_reverse(spec, binary));
        }
        TunnelSpec::RatholeService(spec) => {
            compile_rathole(spec, binary, state_dir, compilation)?;
        }
        TunnelSpec::TrustTunnelEndpoint(spec) => {
            compilation
                .processes
                .push(compile_trusttunnel_endpoint(spec, binary));
        }
    }
    Ok(())
}

fn compile_hpn_server(spec: &HpnSshServer, binary: &Path) -> ProcessSpec {
    ProcessSpec {
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
    }
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
    append_host_key_options(&mut argv, spec);
    append_identity_options(&mut argv, spec);
    append_forward_option(&mut argv, spec);
    argv.push(format!("{}@{}", spec.peer.user, spec.peer.host));
    ProcessSpec {
        id: spec.id.clone(),
        argv,
        env: BTreeMap::new(),
        workdir: None,
        restart: spec.restart.clone(),
    }
}

fn append_host_key_options(argv: &mut Vec<String>, spec: &HpnSshForward) {
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
}

fn append_identity_options(argv: &mut Vec<String>, spec: &HpnSshForward) {
    if let Some(identity) = &spec.peer.identity_file {
        argv.extend(["-i".to_owned(), identity.display().to_string()]);
    }
    for option in &spec.peer.extra_options {
        argv.extend(["-o".to_owned(), option.clone()]);
    }
}

fn append_forward_option(argv: &mut Vec<String>, spec: &HpnSshForward) {
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
    let listener = render_wstunnel_listener(spec);
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

fn render_wstunnel_listener(spec: &WstunnelReverse) -> String {
    match spec.remote_listener.scheme {
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
    }
}

fn compile_rathole(
    spec: &RatholeService,
    binary: &Path,
    state_dir: &Path,
    compilation: &mut TunnelCompilation,
) -> Result<(), String> {
    let path = state_dir
        .join("generated")
        .join(format!("{}.toml", spec.id));
    compilation.generated_files.push(GeneratedFile {
        path: path.clone(),
        content: render_rathole(spec),
    });
    compilation.processes.push(ProcessSpec {
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
        add_rathole_server_listeners(spec, &mut compilation.listeners)?;
    }
    Ok(())
}

fn add_rathole_server_listeners(
    spec: &RatholeService,
    listeners: &mut Vec<ListenerClaim>,
) -> Result<(), String> {
    let control = spec
        .control
        .bind
        .as_ref()
        .expect("validated rathole server control bind");
    listeners.push(listener_claim(&spec.id, control)?);
    for service in &spec.services {
        let bind = service
            .bind
            .as_ref()
            .expect("validated rathole server service bind");
        listeners.push(listener_claim(&spec.id, bind)?);
    }
    Ok(())
}

fn compile_trusttunnel_endpoint(spec: &TrustTunnelEndpoint, binary: &Path) -> ProcessSpec {
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
    ProcessSpec {
        id: spec.id.clone(),
        argv,
        env: BTreeMap::new(),
        workdir: None,
        restart: spec.restart.clone(),
    }
}

fn render_rathole(spec: &RatholeService) -> String {
    let mut output = render_rathole_control(spec);
    render_rathole_transport(&mut output, spec);
    for service in &spec.services {
        render_rathole_service(&mut output, spec, service);
    }
    output
}

fn render_rathole_control(spec: &RatholeService) -> String {
    match spec.role {
        RatholeRole::Server => format!(
            "[server]\nbind_addr = {}\n",
            toml_string(spec.control.bind.as_ref().expect("validated server bind"))
        ),
        RatholeRole::Client => format!(
            "[client]\nremote_addr = {}\n",
            toml_string(
                spec.control
                    .remote
                    .as_ref()
                    .expect("validated client remote")
            )
        ),
    }
}

fn render_rathole_transport(output: &mut String, spec: &RatholeService) {
    let Some(transport) = &spec.transport else {
        return;
    };
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

fn render_rathole_service(
    output: &mut String,
    spec: &RatholeService,
    service: &super::RatholeServiceMapping,
) {
    let header = match spec.role {
        RatholeRole::Server => format!("\n[server.services.{}]\n", service.id),
        RatholeRole::Client => format!("\n[client.services.{}]\n", service.id),
    };
    output.push_str(&header);
    output.push_str(&format!("token = {}\n", toml_string(&service.token)));
    let (key, value) = match spec.role {
        RatholeRole::Server => (
            "bind_addr",
            service
                .bind
                .as_ref()
                .expect("validated server service bind"),
        ),
        RatholeRole::Client => (
            "local_addr",
            service
                .local
                .as_ref()
                .expect("validated client service local"),
        ),
    };
    output.push_str(&format!("{key} = {}\n", toml_string(value)));
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
