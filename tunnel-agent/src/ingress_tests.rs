use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdin, ChildStdout};

fn route() -> PublicPort {
    PublicPort {
        address: Ipv4Addr::new(192, 0, 2, 1),
        port: 8071,
        primary: 48072,
        fallback: Some(48071),
        tls_server_name: "origin.example.com".to_owned(),
        cluster_id: "930fad72-e7d4-4311-8711-591bfaf5b64f".to_owned(),
        lane_id: "4875afdc-a395-4afc-8db1-4cbda27c718a".to_owned(),
        fallback_lane_id: Some("b9a91053-7bb9-431b-a521-52962f205c15".to_owned()),
    }
}

#[test]
fn never_switches_to_an_unready_backend_and_retains_the_current_rule() {
    let configured = route();
    let mut misses = 0;
    let fallback_ready = |port, _host: &str| port == 48071;
    assert_eq!(
        choose_target(&configured, None, &mut misses, fallback_ready),
        Some(48071)
    );
    assert_eq!(
        choose_target(&configured, Some(48071), &mut misses, |_, _| false),
        Some(48071)
    );
    assert_eq!(
        choose_target(&configured, Some(48071), &mut misses, |_, _| true),
        Some(48072)
    );
    assert_eq!(misses, 0);
    for _ in 0..2 {
        assert_eq!(
            choose_target(&configured, Some(48072), &mut misses, fallback_ready),
            Some(48072)
        );
    }
    assert_eq!(
        choose_target(&configured, Some(48072), &mut misses, fallback_ready),
        Some(48071)
    );
}

#[test]
fn rejects_duplicate_claims_and_non_dns_probe_names() {
    let mut configured = route();
    assert!(valid(&[configured.clone(), configured.clone()]).is_err());
    configured.tls_server_name = "127.0.0.1".to_owned();
    assert!(valid(&[configured]).is_err());
}

#[test]
#[ignore = "requires isolated network and mount namespaces with a private /run/netns"]
fn real_nat_transaction_changes_only_the_owned_chain() {
    assert!(available());
    configure_network();
    let _servers = [serve(48071), serve(48072)];
    let external = [
        "-t",
        "nat",
        "-A",
        "PREROUTING",
        "-p",
        "tcp",
        "--dport",
        "8071",
        "-j",
        "DNAT",
        "--to-destination",
        "127.0.0.1:48071",
    ];
    let before = kernel_rules().unwrap();
    assert!(!before.exists);
    iptables(&external).unwrap();
    let mut rule = Rule {
        address: Ipv4Addr::new(192, 0, 2, 1),
        port: 8071,
        interface: "eth0".to_owned(),
        target: 48071,
    };
    allow_loopback_dnat(&rule.interface).unwrap();
    let mut established = TestClient::connect();
    assert_eq!(established.exchange(), "48071");
    restore(&nat_transaction(&before, std::slice::from_ref(&rule))).unwrap();
    let seeded = kernel_rules().unwrap();
    assert!(seeded.exists && seeded.hooked);
    assert_eq!(seeded.rules, vec![rule.clone()]);
    rule.target = 48072;
    restore(&nat_transaction(&seeded, std::slice::from_ref(&rule))).unwrap();
    let switched = kernel_rules().unwrap();
    assert_eq!(switched.rules, vec![rule.clone()]);
    assert_eq!(TestClient::connect().exchange(), "48072");
    assert_eq!(established.exchange(), "48071");
    let raw = iptables(&["-t", "nat", "-S", "PREROUTING"]).unwrap();
    let mut rules = raw.lines().filter(|line| line.starts_with("-A"));
    assert_eq!(rules.next(), Some("-A PREROUTING -j TA_PUBLIC_INGRESS"));
    assert!(rules.next().unwrap().ends_with("127.0.0.1:48071"));
    rule.target = 48071;
    let broken =
        nat_transaction(&switched, &[rule]).replace("COMMIT", "-D NO_SUCH_CHAIN 1\nCOMMIT");
    assert!(restore(&broken).is_err());
    assert_eq!(kernel_rules().unwrap().rules[0].target, 48072);
    assert_eq!(TestClient::connect().exchange(), "48072");
}

fn configure_network() {
    for args in [
        "netns add ingress-client",
        "link set lo up",
        "link add eth0 type veth peer name ingress-peer",
        "link set ingress-peer netns ingress-client",
        "address add 192.0.2.1/24 dev eth0",
        "link set eth0 up",
        "-n ingress-client address add 192.0.2.2/24 dev ingress-peer",
        "-n ingress-client link set ingress-peer up",
    ] {
        assert!(
            Command::new("ip")
                .args(args.split_whitespace())
                .status()
                .unwrap()
                .success(),
            "{args}"
        );
    }
}

fn serve(port: u16) -> std::thread::JoinHandle<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            std::thread::spawn(move || {
                let mut connection = BufReader::new(stream);
                let mut line = String::new();
                while connection.read_line(&mut line).unwrap_or_default() != 0 {
                    if writeln!(connection.get_mut(), "{port}").is_err() {
                        break;
                    }
                    line.clear();
                }
            });
        }
    })
}

struct TestClient {
    process: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl TestClient {
    fn connect() -> Self {
        let mut process = Command::new("ip")
            .args([
                "netns",
                "exec",
                "ingress-client",
                "timeout",
                "5",
                "bash",
                "-ec",
                r#"exec 3<>/dev/tcp/192.0.2.1/8071
                   while read -r request; do
                     printf '%s\n' "$request" >&3
                     read -r reply <&3
                     printf '%s\n' "$reply"
                   done"#,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = process.stdin.take().unwrap();
        let output = BufReader::new(process.stdout.take().unwrap());
        Self {
            process,
            input,
            output,
        }
    }

    fn exchange(&mut self) -> String {
        writeln!(self.input, "test").unwrap();
        let mut response = String::new();
        self.output.read_line(&mut response).unwrap();
        response.trim().to_owned()
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
