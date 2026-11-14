use std::net::IpAddr;

use serde::Deserialize;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_MAX_ACTIVE_LANES: u8 = 1;
const DEFAULT_BUNDLE_POLL_INTERVAL_SECS: u64 = 300;
const MAX_LANES: usize = 16;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_max_active_lanes")]
    pub max_active_lanes: u8,
    #[serde(default)]
    pub lanes: Vec<AgentLane>,
    #[serde(default)]
    pub bundle_seed: Option<BundleSeed>,
}

impl Default for AgentSpec {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval_secs(),
            max_active_lanes: default_max_active_lanes(),
            lanes: Vec::new(),
            bundle_seed: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleSeed {
    /// A locally exposed management lane ending at a foreign seed service.
    pub source: String,
    #[serde(default = "default_bundle_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

impl AgentSpec {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentLane {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub transport: AgentTransport,
    pub direction: AgentDirection,
    pub priority: u16,
    pub peer: AgentPeer,
    pub local_listener: AgentListener,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransport {
    HpnSsh,
    Wstunnel,
    Rathole,
    Trusttunnel,
}

impl AgentTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HpnSsh => "hpnssh",
            Self::Wstunnel => "wstunnel",
            Self::Rathole => "rathole",
            Self::Trusttunnel => "trusttunnel",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentDirection {
    ForeignToDomestic,
    DomesticToForeign,
}

impl AgentDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ForeignToDomestic => "foreign_to_domestic",
            Self::DomesticToForeign => "domestic_to_foreign",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentPeer {
    pub node_id: String,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentListener {
    pub bind: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct AgentLanePlan {
    pub id: String,
    pub selected: bool,
    pub transport: AgentTransport,
    pub direction: AgentDirection,
    pub priority: u16,
    pub peer: AgentPeer,
    pub local_listener: AgentListener,
}

pub fn validate(spec: &AgentSpec) -> Result<(), String> {
    if !(5..=3_600).contains(&spec.poll_interval_secs) {
        return Err("agent.poll_interval_secs must be between 5 and 3600".to_owned());
    }
    if spec.max_active_lanes == 0 || usize::from(spec.max_active_lanes) > MAX_LANES {
        return Err(format!(
            "agent.max_active_lanes must be between 1 and {MAX_LANES}"
        ));
    }
    if spec.lanes.len() > MAX_LANES {
        return Err(format!("agent.lanes supports at most {MAX_LANES} lanes"));
    }
    if let Some(seed) = &spec.bundle_seed {
        validate_bundle_seed(seed)?;
    }

    let mut ids = std::collections::HashSet::new();
    let mut priorities = std::collections::HashSet::new();
    for lane in &spec.lanes {
        validate_lane(lane)?;
        if !ids.insert(&lane.id) {
            return Err(format!("duplicate agent lane id {:?}", lane.id));
        }
        if lane.enabled && !priorities.insert(lane.priority) {
            return Err(format!(
                "enabled agent lanes must have unique priority {}; use an explicit order",
                lane.priority
            ));
        }
    }
    Ok(())
}

fn validate_bundle_seed(seed: &BundleSeed) -> Result<(), String> {
    if !(30..=3_600).contains(&seed.poll_interval_secs) {
        return Err("agent.bundle_seed.poll_interval_secs must be between 30 and 3600".to_owned());
    }
    let Some(authority) = seed.source.strip_prefix("http://") else {
        return Err("agent.bundle_seed.source must be a loopback HTTP URL".to_owned());
    };
    let Some((host, port)) = authority.rsplit_once(':') else {
        return Err("agent.bundle_seed.source must include a loopback port".to_owned());
    };
    if !matches!(host, "127.0.0.1" | "[::1]")
        || port
            .parse::<u16>()
            .ok()
            .filter(|port| *port >= 1024)
            .is_none()
    {
        return Err("agent.bundle_seed.source must use an unprivileged loopback port".to_owned());
    }
    Ok(())
}

pub fn compile_lanes(spec: &AgentSpec) -> Vec<AgentLanePlan> {
    let mut lanes: Vec<_> = spec
        .lanes
        .iter()
        .filter(|lane| lane.enabled)
        .map(|lane| AgentLanePlan {
            id: lane.id.clone(),
            selected: false,
            transport: lane.transport,
            direction: lane.direction,
            priority: lane.priority,
            peer: lane.peer.clone(),
            local_listener: lane.local_listener.clone(),
        })
        .collect();
    lanes.sort_by_key(|lane| lane.priority);
    for lane in lanes.iter_mut().take(usize::from(spec.max_active_lanes)) {
        lane.selected = true;
    }
    lanes
}

fn validate_lane(lane: &AgentLane) -> Result<(), String> {
    if lane.id.len() != 17 || !lane.id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(format!(
            "agent lane id {:?} must be a 17-character base62 hash id",
            lane.id
        ));
    }
    if !valid_uuid(&lane.peer.node_id) {
        return Err(format!(
            "agent lane {:?} peer.node_id must be a UUID",
            lane.id
        ));
    }
    if !valid_host(&lane.peer.host) || lane.peer.port == 0 {
        return Err(format!(
            "agent lane {:?} peer.host and peer.port are required",
            lane.id
        ));
    }
    if !matches!(lane.local_listener.bind.as_str(), "127.0.0.1" | "::1")
        || lane.local_listener.port < 1024
    {
        return Err(format!(
            "agent lane {:?} local_listener must use 127.0.0.1 or ::1 and an unprivileged port",
            lane.id
        ));
    }
    Ok(())
}

fn default_poll_interval_secs() -> u64 {
    DEFAULT_POLL_INTERVAL_SECS
}

fn default_max_active_lanes() -> u8 {
    DEFAULT_MAX_ACTIVE_LANES
}

fn default_bundle_poll_interval_secs() -> u64 {
    DEFAULT_BUNDLE_POLL_INTERVAL_SECS
}

fn default_enabled() -> bool {
    true
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            (matches!(index, 8 | 13 | 18 | 23) && character == '-') || character.is_ascii_hexdigit()
        })
}

fn valid_host(value: &str) -> bool {
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(id: &str) -> AgentLane {
        AgentLane {
            id: id.to_owned(),
            enabled: true,
            transport: AgentTransport::Wstunnel,
            direction: AgentDirection::ForeignToDomestic,
            priority: 10,
            peer: AgentPeer {
                node_id: "d6caf939-b959-4471-918f-2e05ca282a37".to_owned(),
                host: "de-1.example.net".to_owned(),
                port: 4443,
            },
            local_listener: AgentListener {
                bind: "127.0.0.1".to_owned(),
                port: 42_022,
            },
        }
    }

    #[test]
    fn selects_only_the_best_ranked_eligible_lanes() {
        let mut lower = lane("8NdSLH0jy7Af3Gu9h");
        lower.priority = 5;
        let spec = AgentSpec {
            poll_interval_secs: 30,
            max_active_lanes: 1,
            lanes: vec![lane("9NdSLH0jy7Af3Gu9h"), lower],
            bundle_seed: None,
        };
        validate(&spec).expect("valid agent lanes");
        let lanes = compile_lanes(&spec);
        assert_eq!(lanes[0].priority, 5);
        assert!(lanes[0].selected);
        assert!(!lanes[1].selected);
    }

    #[test]
    fn rejects_semantic_ids_and_public_listeners() {
        let mut semantic = lane("hpn-primary");
        assert!(
            validate(&AgentSpec {
                lanes: vec![semantic.clone()],
                ..AgentSpec::default()
            })
            .is_err()
        );
        semantic.id = "8NdSLH0jy7Af3Gu9h".to_owned();
        semantic.local_listener.bind = "0.0.0.0".to_owned();
        assert!(
            validate(&AgentSpec {
                lanes: vec![semantic],
                ..AgentSpec::default()
            })
            .is_err()
        );
    }

    #[test]
    fn accepts_only_loopback_bundle_seed_sources() {
        let mut spec = AgentSpec {
            bundle_seed: Some(BundleSeed {
                source: "http://127.0.0.1:42022".to_owned(),
                poll_interval_secs: 300,
            }),
            ..AgentSpec::default()
        };
        validate(&spec).expect("valid loopback source");
        spec.bundle_seed.as_mut().expect("seed").source = "https://seed.example:42022".to_owned();
        assert!(validate(&spec).is_err());
    }
}
