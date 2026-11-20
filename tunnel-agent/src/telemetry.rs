use std::{
    collections::HashMap,
    fs,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::{StatusCode, blocking::Client, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::{
    agent_lane::{AgentLane, AgentLaneRole},
    agent_spec,
    identity::valid_uuid,
    manifest::Config,
    release,
    transport_identity::TransportKeys,
    tunnel_probes,
};

const MAX_LANES: usize = 16;
const MAX_RELAY_REPORTS: usize = 64;
const MAX_TELEMETRY_BYTES: usize = 16_384;
const MAX_RATE: u64 = 10_000_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TelemetryReport {
    pub(crate) sampled_at: u64,
    pub(crate) host: HostTelemetry,
    pub(crate) lanes: Vec<LaneTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) transport_keys: Option<TransportKeys>,
    #[serde(default)]
    pub(crate) probe_jobs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) probe: Option<crate::probe_job::Progress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) probe_job_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct HostTelemetry {
    pub(crate) cpu_milli_percent: u32,
    pub(crate) memory_milli_percent: u32,
    pub(crate) load_1_milli: u32,
    pub(crate) rx_bytes_per_sec: u64,
    pub(crate) tx_bytes_per_sec: u64,
    pub(crate) agent_rss_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LaneTelemetry {
    pub(crate) id: String,
    pub(crate) state: LaneState,
    pub(crate) probe_ms: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LaneState {
    Healthy,
    Degraded,
    Unknown,
}

pub(crate) struct Reporter {
    previous: Option<HostCounters>,
    transport_keys: TransportKeys,
    state: PathBuf,
}

#[derive(Clone, Copy)]
struct HostCounters {
    sampled_at: u64,
    cpu: CpuCounters,
    network: NetworkCounters,
}

#[derive(Clone, Copy)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

#[derive(Clone, Copy)]
struct NetworkCounters {
    received: u64,
    transmitted: u64,
}

impl Reporter {
    pub(crate) fn new(transport_keys: TransportKeys, state: PathBuf) -> Self {
        Self {
            previous: None,
            transport_keys,
            state,
        }
    }

    pub(crate) fn sample(&mut self, config: &Config) -> Result<Option<TelemetryReport>, String> {
        let current = HostCounters {
            sampled_at: unix_timestamp()?,
            cpu: read_cpu_counters()?,
            network: read_network_counters()?,
        };
        let Some(previous) = self.previous.replace(current) else {
            return Ok(None);
        };
        let elapsed = current.sampled_at.saturating_sub(previous.sampled_at);
        if elapsed == 0 || current.cpu.total <= previous.cpu.total {
            return Ok(None);
        }
        Ok(Some(TelemetryReport {
            sampled_at: current.sampled_at,
            host: HostTelemetry {
                cpu_milli_percent: cpu_percent(previous.cpu, current.cpu),
                memory_milli_percent: memory_percent()?,
                load_1_milli: load_average()?,
                rx_bytes_per_sec: rate(
                    previous.network.received,
                    current.network.received,
                    elapsed,
                ),
                tx_bytes_per_sec: rate(
                    previous.network.transmitted,
                    current.network.transmitted,
                    elapsed,
                ),
                agent_rss_bytes: agent_rss_bytes()?,
            },
            lanes: observed_lanes(config),
            transport_keys: Some(self.transport_keys.clone()),
            probe_jobs: true,
            probe: crate::probe_job::report(&self.state, config.probe_job.as_ref()),
            probe_job_id: config.probe_job.as_ref().map(|job| job.id.clone()),
        }))
    }
}

pub(crate) fn send_direct(
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    report: &TelemetryReport,
) -> Result<(), String> {
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/telemetry",
        api_url.trim_end_matches('/')
    );
    let response = client
        .post(endpoint)
        .bearer_auth(node_token)
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(report, "node telemetry")?)
        .send()
        .map_err(|error| format!("cannot report node telemetry: {error}"))?;
    successful(response.status(), "node telemetry")
}

pub(crate) fn send_domestic(
    source: &str,
    node_id: &str,
    report: &TelemetryReport,
) -> Result<(), String> {
    let endpoint = format!("{}/v1/telemetry/{node_id}", source.trim_end_matches('/'));
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("cannot build domestic telemetry client: {error}"))?;
    let response = client
        .post(endpoint)
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(report, "domestic telemetry")?)
        .send()
        .map_err(|error| format!("cannot send domestic telemetry to relay: {error}"))?;
    successful(response.status(), "domestic telemetry")
}

pub(crate) fn report_relay_telemetry(
    client: &Client,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    state_dir: &Path,
) -> Result<(), String> {
    let reports = read_relay_reports(&state_dir.join("relay-telemetry"))?;
    let report_path = state_dir.join("relay-telemetry-reported.json");
    let mut sent = read_sent_reports(&report_path)?;
    let pending: Vec<_> = reports
        .into_iter()
        .filter(|(domestic_id, report)| {
            sent.get(domestic_id)
                .is_none_or(|seen| report.sampled_at > *seen)
        })
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let payload = serde_json::json!({
        "reports": pending.iter().map(|(node_id, report)| serde_json::json!({
            "node_id": node_id,
            "telemetry": report,
        })).collect::<Vec<_>>(),
    });
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/relay-telemetry",
        api_url.trim_end_matches('/')
    );
    let response = client
        .post(endpoint)
        .bearer_auth(node_token)
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(&payload, "relay telemetry")?)
        .send()
        .map_err(|error| format!("cannot relay telemetry: {error}"))?;
    successful(response.status(), "relay telemetry")?;
    for (domestic_id, report) in pending {
        sent.insert(domestic_id, report.sampled_at);
    }
    let contents = serde_json::to_vec(&sent)
        .map_err(|error| format!("cannot serialize relay telemetry state: {error}"))?;
    release::write_atomic(&report_path, &contents)
}

pub(crate) fn parse_relay_report(bytes: &[u8]) -> Result<TelemetryReport, String> {
    if bytes.is_empty() || bytes.len() > MAX_TELEMETRY_BYTES {
        return Err("relay telemetry payload has invalid size".to_owned());
    }
    let report: TelemetryReport = serde_json::from_slice(bytes)
        .map_err(|error| format!("cannot parse relay telemetry: {error}"))?;
    valid_report(&report)
        .then_some(report)
        .ok_or_else(|| "relay telemetry payload is invalid".to_owned())
}

fn successful(status: StatusCode, label: &str) -> Result<(), String> {
    status
        .is_success()
        .then_some(())
        .ok_or_else(|| format!("{label} request returned HTTP {status}"))
}

fn json_body(value: &impl Serialize, label: &str) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|error| format!("cannot encode {label}: {error}"))
}

fn observed_lanes(config: &Config) -> Vec<LaneTelemetry> {
    let selected: std::collections::HashSet<_> = agent_spec::selected_lanes(&config.agent)
        .into_iter()
        .filter(|lane| lane.selected)
        .map(|lane| lane.id)
        .collect();
    let mut lanes: Vec<_> = config
        .agent
        .lanes
        .iter()
        .filter(|lane| selected.contains(&lane.id))
        .map(probe_lane)
        .collect();
    lanes.extend(tunnel_probes::local_probes(config).take(MAX_LANES.saturating_sub(lanes.len())));
    lanes
}

fn probe_lane(lane: &AgentLane) -> LaneTelemetry {
    let started = Instant::now();
    let endpoint = lane_endpoint(lane);
    match endpoint {
        Some(endpoint) => probe_tcp(
            lane.id.clone(),
            resolve_endpoint(&endpoint),
            started,
            Duration::from_secs(2),
        ),
        None => LaneTelemetry {
            id: lane.id.clone(),
            state: LaneState::Unknown,
            probe_ms: None,
        },
    }
}

pub(crate) fn probe_tcp(
    id: String,
    address: Option<SocketAddr>,
    started: Instant,
    timeout: Duration,
) -> LaneTelemetry {
    let connected =
        address.is_some_and(|value| TcpStream::connect_timeout(&value, timeout).is_ok());
    LaneTelemetry {
        id,
        state: if connected {
            LaneState::Healthy
        } else {
            LaneState::Degraded
        },
        probe_ms: connected.then(|| started.elapsed().as_millis().min(60_000) as u32),
    }
}

fn lane_endpoint(lane: &AgentLane) -> Option<String> {
    match lane.role {
        AgentLaneRole::Server => lane
            .hpnssh
            .server
            .as_ref()
            .map(|server| format!("127.0.0.1:{}", server.port)),
        AgentLaneRole::Client => Some(format!("{}:{}", lane.peer.host, lane.peer.port)),
    }
}

fn resolve_endpoint(endpoint: &str) -> Option<std::net::SocketAddr> {
    endpoint.to_socket_addrs().ok()?.next()
}

fn read_cpu_counters() -> Result<CpuCounters, String> {
    let line = fs::read_to_string("/proc/stat")
        .map_err(|error| format!("cannot read /proc/stat: {error}"))?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    let counters: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|error| format!("cannot parse /proc/stat: {error}"))?;
    if counters.len() < 5 {
        return Err("/proc/stat has too few CPU counters".to_owned());
    }
    Ok(CpuCounters {
        total: counters.iter().sum(),
        idle: counters[3].saturating_add(counters[4]),
    })
}

fn read_network_counters() -> Result<NetworkCounters, String> {
    let data = fs::read_to_string("/proc/net/dev")
        .map_err(|error| format!("cannot read /proc/net/dev: {error}"))?;
    data.lines()
        .skip(2)
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim() != "lo")
        .try_fold(
            NetworkCounters {
                received: 0,
                transmitted: 0,
            },
            |mut counters, (_, values)| {
                let values: Vec<u64> = values
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|error| format!("cannot parse /proc/net/dev: {error}"))?;
                if values.len() < 9 {
                    return Err("/proc/net/dev has too few interface counters".to_owned());
                }
                counters.received = counters.received.saturating_add(values[0]);
                counters.transmitted = counters.transmitted.saturating_add(values[8]);
                Ok(counters)
            },
        )
}

fn memory_percent() -> Result<u32, String> {
    let values = meminfo()?;
    let total = *values
        .get("MemTotal")
        .ok_or_else(|| "MemTotal is missing".to_owned())?;
    let available = *values
        .get("MemAvailable")
        .ok_or_else(|| "MemAvailable is missing".to_owned())?;
    Ok(((total.saturating_sub(available)).saturating_mul(100_000) / total.max(1)) as u32)
}

fn agent_rss_bytes() -> Result<u64, String> {
    let values = meminfo_from("/proc/self/status")?;
    Ok(values
        .get("VmRSS")
        .copied()
        .unwrap_or_default()
        .saturating_mul(1024))
}

fn meminfo() -> Result<HashMap<String, u64>, String> {
    meminfo_from("/proc/meminfo")
}

fn meminfo_from(path: &str) -> Result<HashMap<String, u64>, String> {
    let data = fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    Ok(data
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(key, value)| {
            value
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
                .map(|number| (key.to_owned(), number))
        })
        .collect())
}

fn load_average() -> Result<u32, String> {
    let value = fs::read_to_string("/proc/loadavg")
        .map_err(|error| format!("cannot read /proc/loadavg: {error}"))?;
    let load = value
        .split_whitespace()
        .next()
        .ok_or_else(|| "/proc/loadavg is empty".to_owned())?
        .parse::<f64>()
        .map_err(|error| format!("cannot parse /proc/loadavg: {error}"))?;
    Ok(load
        .mul_add(1000.0, 0.0)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32)
}

fn cpu_percent(previous: CpuCounters, current: CpuCounters) -> u32 {
    let total = current.total.saturating_sub(previous.total).max(1);
    let active = total.saturating_sub(current.idle.saturating_sub(previous.idle));
    (active.saturating_mul(100_000) / total).min(100_000) as u32
}

fn rate(previous: u64, current: u64, elapsed: u64) -> u64 {
    current
        .saturating_sub(previous)
        .saturating_div(elapsed.max(1))
        .min(MAX_RATE)
}

fn unix_timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
}

fn read_relay_reports(directory: &Path) -> Result<HashMap<String, TelemetryReport>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(format!("cannot read relay telemetry: {error}")),
    };
    let mut reports = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read relay telemetry entry: {error}"))?;
        let Some(node_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|value| valid_uuid(value))
            .map(str::to_owned)
        else {
            continue;
        };
        if let Ok(report) = parse_relay_report(
            &fs::read(entry.path())
                .map_err(|error| format!("cannot read relay telemetry {node_id}: {error}"))?,
        ) {
            if reports.len() == MAX_RELAY_REPORTS {
                return Err("relay telemetry has too many entries".to_owned());
            }
            reports.insert(node_id, report);
        }
    }
    Ok(reports)
}

fn read_sent_reports(path: &Path) -> Result<HashMap<String, u64>, String> {
    match fs::read(path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("cannot parse relay telemetry state: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(format!("cannot read relay telemetry state: {error}")),
    }
}

#[path = "telemetry_validate.rs"]
mod validation;
use validation::valid_report;
