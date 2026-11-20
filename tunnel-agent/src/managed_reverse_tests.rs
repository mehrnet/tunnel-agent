use std::{
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::{
    manifest::{self, CompiledPlan},
    release::unique_suffix,
    runtime, transport_identity,
};

const WAIT: Duration = Duration::from_millis(100);

struct Fixture<'a> {
    bundle: &'a Path,
    transport: &'a str,
    listener_port: u16,
    control_port: u16,
    target_port: u16,
}

impl Fixture<'_> {
    fn document(&self, role: &str, local: &str, peer: &str) -> Value {
        let mut listener = json!({ "bind": "127.0.0.1", "port": self.listener_port });
        if role == "client" {
            listener["target"] = json!({ "host": "127.0.0.1", "port": self.target_port });
        }
        json!({
            "version": 3, "bundle": { "root": self.bundle },
            "tunnels": [{
                "contract": "managed.reverse.v1", "id": "owned-reverse-test", "enabled": true,
                "transport": self.transport, "role": role, "scheme": "tcp",
                "control_port": self.control_port, "listener": listener, "peer_host": "127.0.0.1",
                "local_key": local, "peer_key": peer, "generation": "a".repeat(64),
            }],
        })
    }

    fn plan(&self, state: &Path, role: &str, local: &str, peer: &str) -> CompiledPlan {
        let config = manifest::parse_config(&self.document(role, local, peer).to_string()).unwrap();
        manifest::compile(&config, state).unwrap()
    }
}

#[test]
fn validates_managed_claims_without_identity_files_and_rejects_unsafe_settings() {
    let fixture = Fixture {
        bundle: Path::new("/opt/tunnel-agent/current"),
        transport: "rathole",
        listener_port: 18071,
        control_port: 18443,
        target_port: 8000,
    };
    let local = STANDARD.encode([1; 32]);
    let peer = STANDARD.encode([2; 32]);
    let mut raw = fixture.document("server", &local, &peer);
    let config = manifest::parse_config(&raw.to_string()).unwrap();
    let missing = Path::new("/not-provisioned");
    assert!(manifest::compile(&config, missing).is_err());
    let checked = manifest::validate_plan(&config, missing).unwrap();
    assert_eq!(checked.listeners.len(), 2);
    raw["tunnels"][0]["listener"]["bind"] = json!("0.0.0.0");
    assert!(manifest::parse_config(&raw.to_string()).is_err());
    raw["tunnels"][0]["listener"]["bind"] = json!("127.0.0.1");
    raw["tunnels"][0]["scheme"] = json!("socks5");
    assert!(manifest::parse_config(&raw.to_string()).is_err());
}

fn reserve() -> TcpListener {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()
}

fn provision(bundle: &Path, state: &Path, transport: &str) -> String {
    let keys = transport_identity::provision(bundle, state);
    match transport {
        "hpnssh" => keys.hpnssh,
        _ => keys.rathole,
    }
    .expect("bundled transport generated an identity")
}

#[tokio::test]
#[ignore = "requires a bundled HPNSSH/Rathole release; HPNSSH requires an isolated root test environment"]
async fn manages_live_tcp_forwarding_and_closes_listeners_on_disable() {
    let bundle = PathBuf::from(std::env::var_os("TUNNEL_TEST_BUNDLE").expect("TUNNEL_TEST_BUNDLE"));
    let transport = std::env::var("TUNNEL_TEST_TRANSPORT").unwrap_or_else(|_| "rathole".to_owned());
    let root = std::env::temp_dir().join(unique_suffix().unwrap());
    let server_state = root.join("server");
    let client_state = root.join("client");
    let server_key = provision(&bundle, &server_state, &transport);
    let client_key = provision(&bundle, &client_state, &transport);
    assert_eq!(provision(&bundle, &server_state, &transport), server_key);
    let (target, control, listener) = (reserve(), reserve(), reserve());
    let fixture = Fixture {
        bundle: &bundle,
        transport: &transport,
        listener_port: listener.local_addr().unwrap().port(),
        control_port: control.local_addr().unwrap().port(),
        target_port: target.local_addr().unwrap().port(),
    };
    let address = listener.local_addr().unwrap();
    let server_plan = fixture.plan(&server_state, "server", &server_key, &client_key);
    let client_plan = fixture.plan(&client_state, "client", &client_key, &server_key);
    let generated = server_plan.generated_files[0].path.clone();
    if transport == "hpnssh" {
        manifest::write_generated_files(&server_plan).unwrap();
        let checked = std::process::Command::new(bundle.join("hpnssh/sbin/hpnsshd"))
            .args(["-t", "-f"])
            .arg(&generated)
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "hpnsshd rejected generated config: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }
    drop((control, listener));
    let mut server = runtime::start_plan(server_plan).await.unwrap();
    let mut client = runtime::start_plan(client_plan).await.unwrap();
    let permissions = fs::metadata(&generated).unwrap().permissions().mode() & 0o777;
    let forwarded = tokio::task::spawn_blocking(move || exercise_payload(target, address))
        .await
        .unwrap();
    let disabled =
        manifest::parse_config(&json!({"version": 3, "bundle": {"root": bundle}}).to_string())
            .unwrap();
    runtime::reconcile_plan(
        &mut server,
        manifest::compile(&disabled, &server_state).unwrap(),
        false,
    )
    .await;
    let closed = wait_for_closed(address).await;
    runtime::stop_plan(&mut client).await;
    runtime::stop_plan(&mut server).await;
    let removed = !generated.exists();
    fs::remove_dir_all(root).unwrap();
    assert_eq!(permissions, 0o600);
    assert!(forwarded, "{transport} did not forward a TCP payload");
    assert!(
        closed,
        "{transport} kept its TCP listener after server disable"
    );
    assert!(removed, "generated config was retained after disable");
}

fn exercise_payload(target: TcpListener, entrance: SocketAddr) -> bool {
    target.set_nonblocking(true).unwrap();
    let limit = Instant::now() + Duration::from_secs(10);
    while Instant::now() < limit {
        if let Ok(mut ingress) = TcpStream::connect_timeout(&entrance, WAIT) {
            let _ = ingress.set_read_timeout(Some(Duration::from_secs(1)));
            if ingress.write_all(b"lane-request").is_ok() && exchange(&target, &mut ingress) {
                return true;
            }
        }
        std::thread::sleep(WAIT);
    }
    false
}

fn exchange(target: &TcpListener, ingress: &mut TcpStream) -> bool {
    let limit = Instant::now() + Duration::from_secs(1);
    while Instant::now() < limit {
        if let Ok((mut socket, _)) = target.accept() {
            let _ = socket.set_read_timeout(Some(WAIT));
            let mut request = [0; 12];
            let mut reply = [0; 10];
            return socket.read_exact(&mut request).is_ok()
                && &request == b"lane-request"
                && socket.write_all(b"lane-reply").is_ok()
                && ingress.read_exact(&mut reply).is_ok()
                && &reply == b"lane-reply";
        }
        std::thread::sleep(WAIT);
    }
    false
}

async fn wait_for_closed(address: SocketAddr) -> bool {
    for _ in 0..30 {
        if TcpStream::connect_timeout(&address, WAIT).is_err() {
            return true;
        }
        sleep(WAIT).await;
    }
    false
}
