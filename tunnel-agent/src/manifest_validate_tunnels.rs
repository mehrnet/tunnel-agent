use std::collections::HashSet;

use super::{
    HpnDirection, HpnSshForward, HpnSshServer, Listener, RatholeRole, RatholeService,
    RatholeTransportKind, SshPeer, TcpTarget, TunnelListener, TunnelListenerScheme, TunnelSpec,
    WstunnelEndpoint, WstunnelReverse, WstunnelServer,
    manifest_validate::{validate_id, validate_restart},
};

pub(super) fn validate_tunnel(
    tunnel: &TunnelSpec,
    artifacts: &HashSet<&str>,
) -> Result<(), String> {
    validate_id(tunnel.id(), "tunnel")?;
    if !artifacts.contains(tunnel.artifact()) {
        return Err(format!(
            "tunnel {:?} references unknown artifact {:?}",
            tunnel.id(),
            tunnel.artifact()
        ));
    }

    match tunnel {
        TunnelSpec::ManagedReverse(spec) => spec.validate()?,
        TunnelSpec::HpnSshForward(spec) => validate_hpn_forward(spec)?,
        TunnelSpec::HpnSshServer(spec) => validate_hpn_server(spec)?,
        TunnelSpec::WstunnelServer(spec) => validate_wstunnel_server(spec)?,
        TunnelSpec::WstunnelReverse(spec) => validate_wstunnel_reverse(spec)?,
        TunnelSpec::RatholeService(spec) => validate_rathole(spec)?,
    }
    Ok(())
}

fn validate_hpn_forward(spec: &HpnSshForward) -> Result<(), String> {
    validate_restart(&spec.id, &spec.restart)?;
    validate_ssh_peer(&spec.peer, &spec.id)?;
    validate_listener(&spec.listener, &spec.id)?;
    match spec.direction {
        HpnDirection::RemoteSocks | HpnDirection::LocalSocks if spec.listener.target.is_some() => {
            Err(format!(
                "tunnel {:?} SOCKS modes cannot define listener.target",
                spec.id
            ))
        }
        HpnDirection::RemoteTcp if spec.listener.target.is_none() => Err(format!(
            "tunnel {:?} remote_tcp requires listener.target",
            spec.id
        )),
        _ => Ok(()),
    }
}

fn validate_hpn_server(spec: &HpnSshServer) -> Result<(), String> {
    validate_restart(&spec.id, &spec.restart)?;
    if spec.config_file.as_os_str().is_empty() {
        return Err(format!(
            "tunnel {:?} config_file must not be empty",
            spec.id
        ));
    }
    Ok(())
}

fn validate_wstunnel_server(spec: &WstunnelServer) -> Result<(), String> {
    validate_restart(&spec.id, &spec.restart)?;
    validate_listener(&spec.listener, &spec.id)?;
    if spec.listener.target.is_some() {
        return Err(format!(
            "tunnel {:?} server listener cannot define target",
            spec.id
        ));
    }
    validate_wstunnel_path(&spec.path_prefix, &spec.id)?;
    if let Some(port) = spec.fixed_reverse_port
        && (port < 1024 || port == spec.listener.port || !spec.id.starts_with("owned-"))
    {
        return Err(format!(
            "tunnel {:?} has an invalid fixed reverse port",
            spec.id
        ));
    }
    validate_worker_threads(spec.worker_threads, &spec.id)
}

fn validate_wstunnel_reverse(spec: &WstunnelReverse) -> Result<(), String> {
    validate_restart(&spec.id, &spec.restart)?;
    validate_wstunnel_endpoint(&spec.endpoint, &spec.id)?;
    validate_tunnel_listener(&spec.remote_listener, &spec.id)?;
    validate_worker_threads(spec.worker_threads, &spec.id)?;
    if spec.forward_udp
        && (!matches!(spec.remote_listener.scheme, TunnelListenerScheme::Tcp)
            || spec.remote_listener.bind != "127.0.0.1")
    {
        return Err(format!(
            "tunnel {:?} UDP forwarding requires a fixed loopback listener",
            spec.id
        ));
    }
    match spec.remote_listener.scheme {
        TunnelListenerScheme::Socks5 if spec.remote_listener.target.is_some() => Err(format!(
            "tunnel {:?} SOCKS listener cannot define target",
            spec.id
        )),
        TunnelListenerScheme::Tcp if spec.remote_listener.target.is_none() => {
            Err(format!("tunnel {:?} TCP listener requires target", spec.id))
        }
        _ => Ok(()),
    }
}

fn validate_worker_threads(worker_threads: u16, id: &str) -> Result<(), String> {
    if worker_threads == 0 {
        return Err(format!("tunnel {id:?} worker_threads must be positive"));
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
    if !id.starts_with("owned-") {
        return Ok(());
    }
    let prefix = path.strip_prefix('/').unwrap_or("");
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "tunnel {id:?} path_prefix must contain one non-empty URL segment after '/'"
        ));
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
    validate_rathole_control(spec)?;
    validate_rathole_services(spec)
}

fn validate_rathole_control(spec: &RatholeService) -> Result<(), String> {
    match spec.role {
        RatholeRole::Server
            if spec.control.bind.as_deref().unwrap_or_default().is_empty()
                || spec.control.remote.is_some() =>
        {
            Err(format!(
                "tunnel {:?} Rathole server requires control.bind and rejects control.remote",
                spec.id
            ))
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
            Err(format!(
                "tunnel {:?} Rathole client requires control.remote and rejects control.bind",
                spec.id
            ))
        }
        _ => Ok(()),
    }
}

fn validate_rathole_services(spec: &RatholeService) -> Result<(), String> {
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
        validate_rathole_service_address(spec, service)?;
    }
    Ok(())
}

fn validate_rathole_service_address(
    spec: &RatholeService,
    service: &super::RatholeServiceMapping,
) -> Result<(), String> {
    match spec.role {
        RatholeRole::Server
            if service.bind.as_deref().unwrap_or_default().is_empty()
                || service.local.is_some() =>
        {
            Err(format!(
                "tunnel {:?} server service {:?} requires bind and rejects local",
                spec.id, service.id
            ))
        }
        RatholeRole::Client
            if service.local.as_deref().unwrap_or_default().is_empty()
                || service.bind.is_some() =>
        {
            Err(format!(
                "tunnel {:?} client service {:?} requires local and rejects bind",
                spec.id, service.id
            ))
        }
        _ => Ok(()),
    }
}
