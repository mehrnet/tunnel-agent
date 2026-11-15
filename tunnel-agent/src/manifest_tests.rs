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
        agent: AgentSpec::default(),
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
        agent: AgentSpec::default(),
    };
    assert!(validate_config(&config).is_err());
}

#[test]
fn rejects_a_foreign_bundle_seed_consumer() {
    let raw = r#"{
      "version": 3,
      "node_scope": "foreign",
      "bundle": {"root": "/opt/tunnel-agent/current"},
      "agent": {"bundle_seed": {"source": "http://127.0.0.1:42022"}},
      "tunnels": []
    }"#;
    assert!(parse_config(raw).is_err());
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
        agent: AgentSpec::default(),
    };
    let plan = compile(&config, Path::new("/var/lib/tunnel-agent")).expect("valid rathole plan");
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
        agent: AgentSpec::default(),
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
        agent: AgentSpec::default(),
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
fn compiles_an_agent_lane_plan_with_control_processes_only() {
    let raw = r#"{
      "version": 3,
      "node_scope": "foreign",
      "bundle": {"root": "/opt/tunnel-agent/current"},
      "agent": {
        "lanes": [{
          "id": "8NdSLH0jy7Af3Gu9h",
          "transport": "hpnssh",
          "direction": "foreign_to_domestic",
          "role": "client",
          "priority": 10,
          "peer": {
            "node_id": "d6caf939-b959-4471-918f-2e05ca282a37",
            "host": "ir-6.example.net",
            "port": 42001
          },
          "listener": {"bind": "127.0.0.1", "port": 14444},
          "hpnssh": {
            "cluster_id": "4cd57b8f-8f97-4f80-8791-90bba10190d0",
            "client": {
              "user": "root",
              "identity_file": "/state/control.pem",
              "known_hosts_file": "/state/known_hosts",
              "known_host": "[ir-6.example.net]:42001 ssh-ed25519 AAAA",
              "target": {"host": "127.0.0.1", "port": 42022}
            }
          }
        }]
      }
    }"#;
    let config: Config = serde_json::from_str(raw).expect("valid agent schema");
    let plan = compile(&config, Path::new("/unused-state")).expect("valid agent plan");
    assert_eq!(plan.processes.len(), 2);
    assert_eq!(plan.agent_lanes[0].id, "8NdSLH0jy7Af3Gu9h");
    assert!(plan.agent_lanes[0].selected);
    assert_eq!(plan.listeners[0].owner, "agent seed 8NdSLH0jy7Af3Gu9h");
}

#[test]
fn rejects_agent_lane_listener_collision_with_a_local_socks_worker() {
    let raw = r#"{
      "version": 3,
      "node_scope": "domestic",
      "bundle": {"root": "/opt/tunnel-agent/current"},
      "agent": {
        "lanes": [{
          "id": "8NdSLH0jy7Af3Gu9h",
          "transport": "hpnssh",
          "direction": "foreign_to_domestic",
          "role": "server",
          "priority": 10,
          "peer": {
            "node_id": "d6caf939-b959-4471-918f-2e05ca282a37",
            "host": "de-1.example.net",
            "port": 42001
          },
          "listener": {"bind": "127.0.0.1", "port": 14444},
          "hpnssh": {
            "cluster_id": "4cd57b8f-8f97-4f80-8791-90bba10190d0",
            "server": {
              "bind": "0.0.0.0",
              "port": 42001,
              "host_key_file": "/etc/tunnel-agent/host_key",
              "authorized_keys_file": "/etc/tunnel-agent/authorized_keys"
            }
          }
        }]
      },
      "tunnels": [{
        "contract": "hpnssh.forward.v1",
        "id": "customer_socks",
        "artifact": "hpnssh",
        "direction": "local_socks",
        "peer": {
          "host": "peer.example.net",
          "user": "tunnel",
          "port": 2222,
          "known_hosts_file": "/etc/tunnel-agent/known_hosts"
        },
        "listener": {"bind": "127.0.0.1", "port": 14444}
      }]
    }"#;
    let config: Config = serde_json::from_str(raw).expect("valid agent schema");
    assert!(compile(&config, Path::new("/unused-state")).is_err());
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
