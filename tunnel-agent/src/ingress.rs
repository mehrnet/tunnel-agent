use std::{
    collections::{HashMap, HashSet},
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpListener},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::manifest::{Config, GeneratedFile, ListenerClaim, ProcessSpec, RestartPolicy};

const CHAIN: &str = "TA_PUBLIC_INGRESS";
const RETRY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicPort {
    cluster_id: String,
    lane_id: String,
    fallback_lane_id: Option<String>,
    address: Ipv4Addr,
    port: u16,
    primary: u16,
    fallback: Option<u16>,
    tls_server_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rule {
    address: Ipv4Addr,
    port: u16,
    interface: String,
    target: u16,
}

pub(crate) fn available() -> bool {
    (unsafe { libc::geteuid() == 0 })
        && Path::new("/proc/sys/net/ipv4/conf").is_dir()
        && command_version("ip", "-V")
        && command_version("iptables", "--version")
        && command_version("iptables-restore", "--version")
}

fn command_version(name: &str, flag: &str) -> bool {
    Command::new(name)
        .arg(flag)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn valid(routes: &[PublicPort]) -> Result<(), String> {
    if routes.len() > 16 {
        return Err("at most 16 public ports are supported per node".to_owned());
    }
    let mut claims = HashSet::new();
    for route in routes {
        if !crate::identity::valid_uuid(&route.cluster_id)
            || !crate::identity::valid_uuid(&route.lane_id)
            || route
                .fallback_lane_id
                .as_deref()
                .is_some_and(|id| !crate::identity::valid_uuid(id) || id == route.lane_id)
            || route.fallback.is_some() != route.fallback_lane_id.is_some()
            || route.address.is_unspecified()
            || route.address.is_loopback()
            || route.address.is_multicast()
            || route.address.is_broadcast()
            || route.port == 0
            || route.primary < 1024
            || route.primary == route.port
            || route
                .fallback
                .is_some_and(|port| port < 1024 || port == route.primary || port == route.port)
            || !claims.insert((route.address, route.port))
            || !valid_tls_name(&route.tls_server_name)
        {
            return Err("invalid public ingress address, ports, or TLS probe name".to_owned());
        }
    }
    Ok(())
}

fn valid_tls_name(name: &str) -> bool {
    name.contains('.')
        && name.bytes().any(|byte| byte.is_ascii_alphabetic())
        && crate::managed_reverse::valid_host(name)
}

pub(crate) fn compile(
    config: &Config,
    state: &Path,
    workers: &mut Vec<ProcessSpec>,
    files: &mut Vec<GeneratedFile>,
    listeners: &mut Vec<ListenerClaim>,
) -> Result<(), String> {
    let Some(routes) = &config.public_ingress else {
        return Ok(());
    };
    let bundle = config
        .bundle
        .as_ref()
        .ok_or_else(|| "public ingress requires a bundle".to_owned())?;
    let file = state.join("generated/public-ingress.json");
    let contents = serde_json::to_string(routes).map_err(|error| error.to_string())?;
    workers.push(ProcessSpec {
        id: "agent-public-ingress".to_owned(),
        argv: vec![
            bundle.root.join("bin/tunnel-agent").display().to_string(),
            "public-ingress-worker".to_owned(),
            "--config".to_owned(),
            file.display().to_string(),
        ],
        env: Default::default(),
        workdir: None,
        restart: RestartPolicy::default(),
    });
    files.push(GeneratedFile {
        path: file,
        content: contents,
    });
    listeners.extend(routes.iter().map(|route| ListenerClaim {
        owner: format!("public ingress {}", route.port),
        bind: route.address.to_string(),
        port: route.port,
    }));
    Ok(())
}

pub(crate) fn run(config: &Path) -> Result<(), String> {
    let routes: Vec<PublicPort> =
        serde_json::from_slice(&std::fs::read(config).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    valid(&routes)?;
    let mut failures = HashMap::new();
    loop {
        match reconcile(&routes, &mut failures) {
            Ok(()) => {}
            Err(error) => eprintln!("public ingress update postponed: {error}"),
        }
        std::thread::sleep(RETRY);
    }
}

fn reconcile(
    routes: &[PublicPort],
    failures: &mut HashMap<(Ipv4Addr, u16), u8>,
) -> Result<(), String> {
    let kernel = kernel_rules()?;
    let mut desired = Vec::with_capacity(routes.len());
    for route in routes {
        let interface = interface_for(route.address)?;
        let previous = kernel
            .rules
            .iter()
            .find(|rule| rule.address == route.address && rule.port == route.port);
        if previous.is_none() {
            let address = SocketAddr::from((route.address, route.port));
            TcpListener::bind(address)
                .map_err(|error| format!("cannot claim public port {address}: {error}"))?;
        }
        let misses = failures.entry((route.address, route.port)).or_default();
        if let Some(target) =
            choose_target(route, previous.map(|rule| rule.target), misses, tls_ready)
        {
            desired.push(Rule {
                address: route.address,
                port: route.port,
                interface,
                target,
            });
        }
    }
    desired.sort();
    if desired != kernel.rules || (!desired.is_empty() && !kernel.hooked) {
        for rule in &desired {
            allow_loopback_dnat(&rule.interface)?;
        }
        restore(&nat_transaction(&kernel, &desired))?;
        eprintln!("public ingress applied {} mapping(s)", desired.len());
    }
    Ok(())
}

fn choose_target(
    route: &PublicPort,
    current: Option<u16>,
    misses: &mut u8,
    ready: impl Fn(u16, &str) -> bool,
) -> Option<u16> {
    if ready(route.primary, &route.tls_server_name) {
        *misses = 0;
        return Some(route.primary);
    }
    *misses = misses.saturating_add(1);
    if current != Some(route.primary) || *misses >= 3 {
        return route
            .fallback
            .filter(|port| ready(*port, &route.tls_server_name))
            .or(current);
    }
    current
}

fn tls_ready(port: u16, sni: &str) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    reqwest::blocking::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(sni, address)
        .build()
        .and_then(|client| client.head(format!("https://{sni}:{port}/")).send())
        .is_ok()
}

struct Kernel {
    exists: bool,
    hooked: bool,
    rules: Vec<Rule>,
}

fn kernel_rules() -> Result<Kernel, String> {
    let raw = iptables(&["-t", "nat", "-S"])?;
    let exists = raw.lines().any(|line| line == format!("-N {CHAIN}"));
    let hook = format!("-A PREROUTING -j {CHAIN}");
    let hooked = raw.lines().any(|line| line == hook);
    let prefix = format!("-A {CHAIN} ");
    let mut rules = raw
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .map(parse_rule)
        .collect::<Result<Vec<_>, _>>()?;
    rules.sort();
    Ok(Kernel {
        exists,
        hooked,
        rules,
    })
}

fn parse_rule(line: &str) -> Result<Rule, String> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let lookup = |name: &str| {
        tokens
            .windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1])
    };
    (|| {
        Some(Rule {
            address: lookup("-d")?.strip_suffix("/32")?.parse().ok()?,
            port: lookup("--dport")?.parse().ok()?,
            interface: lookup("-i")?.to_owned(),
            target: lookup("--to-destination")?
                .strip_prefix("127.0.0.1:")?
                .parse()
                .ok()?,
        })
    })()
    .ok_or_else(|| "unexpected rule in public ingress chain".to_owned())
}

fn nat_transaction(kernel: &Kernel, routes: &[Rule]) -> String {
    let mut script = "*nat\n".to_owned();
    if !kernel.exists {
        script.push_str(&format!("-N {CHAIN}\n"));
    }
    if !kernel.hooked && !routes.is_empty() {
        script.push_str(&format!("-I PREROUTING 1 -j {CHAIN}\n"));
    }
    script.push_str(&format!("-F {CHAIN}\n"));
    for rule in routes {
        script.push_str(&format!(
            "-A {CHAIN} -i {} -d {}/32 -p tcp --dport {} -j DNAT --to-destination 127.0.0.1:{}\n",
            rule.interface, rule.address, rule.port, rule.target,
        ));
    }
    script.push_str("COMMIT\n");
    script
}

fn interface_for(address: Ipv4Addr) -> Result<String, String> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show"])
        .output()
        .map_err(|error| format!("cannot inspect local addresses: {error}"))?;
    let address = address.to_string();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let entry = line.split_whitespace().take(4).collect::<Vec<_>>();
        if let [_, interface, "inet", cidr] = entry.as_slice()
            && cidr.split('/').next() == Some(address.as_str())
        {
            let interface = interface.split('@').next().unwrap_or_default();
            if !interface.is_empty()
                && interface
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            {
                return Ok(interface.to_owned());
            }
        }
    }
    Err(format!(
        "public ingress address {address} is not assigned to a local interface"
    ))
}

fn allow_loopback_dnat(interface: &str) -> Result<(), String> {
    let sysctl = format!("/proc/sys/net/ipv4/conf/{interface}/route_localnet");
    // Preserve the loopback trust boundary when routing becomes enabled on this interface.
    let rule = ["-i", interface, "-d", "127.0.0.0/8", "-j", "DROP"];
    if iptables(&[["-t", "raw", "-C", "PREROUTING"].as_slice(), &rule].concat()).is_err() {
        iptables(&[["-t", "raw", "-I", "PREROUTING", "1"].as_slice(), &rule].concat())?;
    }
    if std::fs::read_to_string(&sysctl).is_ok_and(|value| value.trim() == "1") {
        return Ok(());
    }
    std::fs::write(sysctl, "1").map_err(|error| format!("cannot enable loopback DNAT: {error}"))
}

fn iptables(args: &[&str]) -> Result<String, String> {
    let output = Command::new("iptables")
        .args(["-w", "3"])
        .args(args)
        .output()
        .map_err(|error| format!("cannot execute iptables: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "iptables rejected operation: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn restore(script: &str) -> Result<(), String> {
    let mut process = Command::new("iptables-restore")
        .args(["--wait", "3", "--noflush"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start iptables-restore: {error}"))?;
    let delivery = process
        .stdin
        .take()
        .expect("piped child input")
        .write_all(script.as_bytes());
    let output = process
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "public ingress transaction rejected: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    delivery.map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
