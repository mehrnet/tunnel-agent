use std::net::IpAddr;

use serde::Deserialize;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_MAX_ACTIVE_LANES: u8 = 1;
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
}

impl Default for AgentSpec {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval_secs(),
            max_active_lanes: default_max_active_lanes(),
            lanes: Vec::new(),
        }
    }
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

    let mut ids = std::collections::HashSet::new();
    let mut priorities = std::collections::HashSet::new();
    let mut enabled = 0usize;
    for lane in &spec.lanes {
        validate_lane(lane)?;
        if !ids.insert(&lane.id) {
            return Err(format!("duplicate agent lane id {:?}", lane.id));
        }
        if lane.enabled {
            enabled += 1;
            if !priorities.insert(lane.priority) {
                return Err(format!(
                    "enabled agent lanes must have unique priority {}; use an explicit order",
                    lane.priority
                ));
            }
        }
    }
    if enabled > usize::from(spec.max_active_lanes) {
        return Err(format!(
            "agent has {enabled} enabled lanes but max_active_lanes is {}",
            spec.max_active_lanes
        ));
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
            transport: lane.transport,
            direction: lane.direction,
            priority: lane.priority,
            peer: lane.peer.clone(),
            local_listener: lane.local_listener.clone(),
        })
        .collect();
    lanes.sort_by_key(|lane| lane.priority);
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
    fn accepts_an_opaque_lane_and_orders_it_by_priority() {
        let mut lower = lane("8NdSLH0jy7Af3Gu9h");
        lower.priority = 5;
        let spec = AgentSpec {
            poll_interval_secs: 30,
            max_active_lanes: 2,
            lanes: vec![lane("9NdSLH0jy7Af3Gu9h"), lower],
        };
        validate(&spec).expect("valid agent lanes");
        assert_eq!(compile_lanes(&spec)[0].priority, 5);
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
}
