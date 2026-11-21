use std::{
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    agent_relays::{
        first_working,
        tests::{config, lane},
    },
    manifest, release, runtime, transport_identity,
};

fn reserve() -> TcpListener {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()
}

fn port(socket: &TcpListener) -> u16 {
    socket.local_addr().unwrap().port()
}

fn make_cert(root: &Path, name: &str, purpose: &str) {
    let ca = format!("{name}-ca");
    openssl(
        root,
        &[
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-nodes",
            "-subj",
            &format!("/CN={ca}"),
            "-keyout",
            &format!("{ca}.key"),
            "-out",
            &format!("{ca}.crt"),
        ],
    );
    openssl(
        root,
        &[
            "req",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-nodes",
            "-subj",
            "/CN=management.example",
            "-keyout",
            &format!("{name}.key"),
            "-out",
            &format!("{name}.csr"),
        ],
    );
    fs::write(
        root.join("ext.cnf"),
        format!("subjectAltName=DNS:management.example\nextendedKeyUsage={purpose}\n"),
    )
    .unwrap();
    openssl(
        root,
        &[
            "x509",
            "-req",
            "-in",
            &format!("{name}.csr"),
            "-CA",
            &format!("{ca}.crt"),
            "-CAkey",
            &format!("{ca}.key"),
            "-CAcreateserial",
            "-extfile",
            "ext.cnf",
            "-out",
            &format!("{name}.crt"),
        ],
    );
}

fn openssl(root: &Path, args: &[&str]) {
    let output = Command::new("openssl")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_tls(root: &Path, server: &mut Value, client: &mut Value) {
    make_cert(root, "server", "serverAuth");
    make_cert(root, "client", "clientAuth");
    for (lane, local, peer) in [(server, "server", "client"), (client, "client", "server")] {
        lane["wstunnel"]["certificate_file"] = json!(root.join(format!("{local}.crt")));
        lane["wstunnel"]["key_file"] = json!(root.join(format!("{local}.key")));
        lane["wstunnel"]["peer_ca_file"] = json!(root.join(format!("{peer}-ca.crt")));
    }
}

fn configure_noise(bundle: &Path, root: &Path, server: &mut Value, client: &mut Value) {
    let server_state = root.join("server");
    let client_state = root.join("client");
    let public = |state| {
        transport_identity::provision(bundle, state)
            .rathole
            .unwrap()
    };
    let (server_key, client_key) = (public(&server_state), public(&client_state));
    server["rathole"]["local_key"] = json!(server_key);
    client["rathole"]["peer_key"] = json!(server_key);
    client["rathole"]["local_key"] = json!(client_key);
    server["rathole"]["peer_key"] = json!(client_key);
}

fn request(target: &TcpListener, address: SocketAddr) -> bool {
    let mut client = match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline {
        match target.accept() {
            Ok((mut stream, _)) => {
                client
                    .set_read_timeout(Some(Duration::from_millis(700)))
                    .unwrap();
                let mut bytes = [0; 4];
                return stream.write_all(b"seed").is_ok()
                    && client.read_exact(&mut bytes).is_ok()
                    && &bytes == b"seed";
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    false
}

#[tokio::test]
#[ignore = "requires release binaries in TUNNEL_TEST_BUNDLE and openssl"]
async fn forwards_management_payload_and_fails_over_across_real_transports() {
    let bundle = PathBuf::from(std::env::var_os("TUNNEL_TEST_BUNDLE").expect("TUNNEL_TEST_BUNDLE"));
    let root = std::env::temp_dir().join(release::unique_suffix().unwrap());
    fs::create_dir(&root).unwrap();
    let target = reserve();
    target.set_nonblocking(true).unwrap();
    let (ws_control, ws_listener) = (reserve(), reserve());
    let (noise_control, noise_listener) = (reserve(), reserve());
    let (mut ws_server, mut ws_client) =
        fixture("wstunnel", 'W', &ws_control, &ws_listener, &target);
    let (mut noise_server, mut noise_client) =
        fixture("rathole", 'R', &noise_control, &noise_listener, &target);
    configure_tls(&root, &mut ws_server, &mut ws_client);
    configure_noise(&bundle, &root, &mut noise_server, &mut noise_client);
    let mut domestic = manifest::parse_config(&config(
        "domestic",
        &bundle,
        2,
        vec![ws_server, noise_server],
    ))
    .unwrap();
    let foreign = manifest::parse_config(&config(
        "foreign",
        &bundle,
        2,
        vec![ws_client, noise_client],
    ))
    .unwrap();
    let server_state = root.join("server");
    let mut server_plan = manifest::compile(&domestic, &server_state).unwrap();
    let mut client_plan = manifest::compile(&foreign, &root.join("client")).unwrap();
    // The echo listener stands in for the real seed service; use the compiled lane unchanged.
    client_plan
        .processes
        .retain(|spec| spec.id.starts_with("agent-lane-"));
    if std::env::var_os("TUNNEL_TEST_WORKER_LOGS").is_some() {
        for spec in server_plan
            .processes
            .iter_mut()
            .chain(client_plan.processes.iter_mut())
        {
            if let Some(arg) = spec.argv.iter().position(|arg| arg == "--log-lvl") {
                spec.argv[arg + 1] = "DEBUG".to_owned();
            }
        }
    }
    let primary = ws_listener.local_addr().unwrap();
    let backup = noise_listener.local_addr().unwrap();
    drop((ws_control, ws_listener, noise_control, noise_listener));
    let mut server = runtime::start_plan(server_plan).await.unwrap();
    let mut client = runtime::start_plan(client_plan).await.unwrap();
    let (ws, noise, target) = tokio::task::spawn_blocking(move || {
        (
            wait_for_payload(&target, primary),
            wait_for_payload(&target, backup),
            target,
        )
    })
    .await
    .unwrap();
    domestic.agent.lanes[0].enabled = false;
    let next = manifest::compile(&domestic, &server_state).unwrap();
    manifest::write_generated_files(&next).unwrap();
    runtime::reconcile_plan(&mut server, next, &[]).await;
    let failover = tokio::task::spawn_blocking(move || {
        first_working(&[primary.to_string(), backup.to_string()], |path| {
            request(&target, path.parse().unwrap())
                .then(|| path.to_owned())
                .ok_or_else(|| "no payload".to_owned())
        })
    })
    .await
    .unwrap();
    runtime::stop_plan(&mut client).await;
    runtime::stop_plan(&mut server).await;
    fs::remove_dir_all(root).unwrap();
    assert!(ws, "WSTunnel failed to forward the management payload");
    assert!(noise, "Rathole failed to forward the management payload");
    assert_eq!(failover.unwrap(), backup.to_string());
}

fn fixture(
    transport: &str,
    id: char,
    control: &TcpListener,
    listener: &TcpListener,
    target: &TcpListener,
) -> (Value, Value) {
    let server = lane(transport, id, port(control), port(listener), "server");
    let mut client = lane(transport, id, port(control), port(listener), "client");
    client[transport]["target"]["port"] = json!(port(target));
    (server, client)
}

fn wait_for_payload(target: &TcpListener, address: SocketAddr) -> bool {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if request(target, address) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}
