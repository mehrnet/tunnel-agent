use serde::Deserialize;

pub use crate::agent_lane::{AgentLane, AgentLanePlan};

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_CONFIG_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_MAX_ACTIVE_LANES: u8 = 1;
const DEFAULT_BUNDLE_POLL_INTERVAL_SECS: u64 = 300;
const MAX_LANES: usize = 16;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    #[serde(default = "default_config_poll_interval_secs")]
    pub config_poll_interval_secs: u64,
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
            config_poll_interval_secs: default_config_poll_interval_secs(),
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

    pub fn lanes_changed(&self, other: &Self) -> bool {
        self.max_active_lanes != other.max_active_lanes || self.lanes != other.lanes
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransport {
    #[serde(rename = "hpnssh")]
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

pub fn validate(spec: &AgentSpec) -> Result<(), String> {
    if !(5..=3_600).contains(&spec.config_poll_interval_secs) {
        return Err("agent.config_poll_interval_secs must be between 5 and 3600".to_owned());
    }
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

pub fn selected_lanes(spec: &AgentSpec) -> Vec<AgentLanePlan> {
    let mut lanes: Vec<_> = spec
        .lanes
        .iter()
        .filter(|lane| lane.enabled)
        .map(|lane| AgentLanePlan {
            id: lane.id.clone(),
            selected: false,
            transport: lane.transport,
            direction: lane.direction,
            role: lane.role,
            priority: lane.priority,
            peer: lane.peer.clone(),
            listener: lane.listener.clone(),
        })
        .collect();
    lanes.sort_by_key(|lane| lane.priority);
    for lane in lanes.iter_mut().take(usize::from(spec.max_active_lanes)) {
        lane.selected = true;
    }
    lanes
}

fn default_poll_interval_secs() -> u64 {
    DEFAULT_POLL_INTERVAL_SECS
}

fn default_config_poll_interval_secs() -> u64 {
    DEFAULT_CONFIG_POLL_INTERVAL_SECS
}

fn default_max_active_lanes() -> u8 {
    DEFAULT_MAX_ACTIVE_LANES
}

fn default_bundle_poll_interval_secs() -> u64 {
    DEFAULT_BUNDLE_POLL_INTERVAL_SECS
}
