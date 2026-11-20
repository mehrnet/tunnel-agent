use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::{
    managed_reverse::ManagedRole,
    manifest::{Config, HpnDirection, Listener, TunnelSpec},
    telemetry::{LaneTelemetry, probe_tcp},
};

pub(crate) fn local_probes(config: &Config) -> impl Iterator<Item = LaneTelemetry> + '_ {
    config.tunnels.iter().filter_map(|tunnel| {
        let listener = match tunnel {
            TunnelSpec::ManagedReverse(spec)
                if spec.enabled && spec.role == ManagedRole::Server =>
            {
                &spec.listener
            }
            TunnelSpec::WstunnelServer(spec) if spec.enabled => &spec.listener,
            TunnelSpec::HpnSshForward(spec)
                if spec.enabled && spec.direction == HpnDirection::LocalSocks =>
            {
                &spec.listener
            }
            _ => return None,
        };
        let address = local_address(listener)?;
        // Alphanumeric IDs also pass validation on older relays.
        let id = format!("tunnel{:x}", Sha256::digest(tunnel.id().as_bytes()));
        Some(probe_tcp(
            id,
            Some(address),
            Instant::now(),
            Duration::from_millis(200),
        ))
    })
}

fn local_address(listener: &Listener) -> Option<SocketAddr> {
    let bind = match listener.bind.parse::<IpAddr>().ok()? {
        IpAddr::V4(bind) if bind.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(bind) if bind.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        bind if bind.is_loopback() => bind,
        _ => return None,
    };
    Some(SocketAddr::new(bind, listener.port))
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use crate::{manifest::parse_config, telemetry::LaneState};

    use super::*;

    #[test]
    fn observes_a_real_listener_and_never_probes_disabled_or_non_local_binds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let tunnel = |id, bind, enabled| {
            serde_json::json!({
                "id": id, "contract": "wstunnel.server.v1", "enabled": enabled,
                "artifact": "wstunnel", "listener": {"bind": bind, "port": port},
                "tls_certificate": "/tmp/cert.pem", "tls_private_key": "/tmp/key.pem", "path_prefix": "/test",
            })
        };
        let config = parse_config(
            &serde_json::json!({
                "version": 3, "bundle": { "root": "/opt/tunnel-agent/current" },
                "tunnels": [
                    tunnel("owned-test", "0.0.0.0", true),
                    tunnel("disabled", "127.0.0.1", false),
                    tunnel("remote", "192.0.2.1", true)
                ]
            })
            .to_string(),
        )
        .unwrap();
        let reports: Vec<_> = local_probes(&config).collect();
        assert_eq!(reports.len(), 1);
        assert!(matches!(reports[0].state, LaneState::Healthy));
        assert!(reports[0].probe_ms.is_some());
        drop(listener);
        let report = local_probes(&config).next().unwrap();
        assert!(matches!(report.state, LaneState::Degraded));
        assert!(report.probe_ms.is_none());
    }
}
