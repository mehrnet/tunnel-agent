use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

use crate::{
    agent_lane::{AgentLane, AgentLaneRole, AgentTarget, LaneCompilation, authority},
    managed_reverse::{ManagedReverse, ManagedRole, ManagedTransport},
    manifest::{
        GeneratedFile, Listener, ListenerClaim, ProcessSpec, TcpTarget, TunnelListenerScheme,
    },
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RatholeLane {
    pub local_key: String,
    pub peer_key: String,
    pub generation: String,
    pub target: Option<AgentTarget>,
}

impl RatholeLane {
    pub(crate) fn validate(&self, lane: &AgentLane) -> Result<(), String> {
        validate_target(lane, self.target.as_ref())?;
        self.contract(lane).validate()
    }

    fn contract(&self, lane: &AgentLane) -> ManagedReverse {
        ManagedReverse {
            id: format!("agent-lane-{}", lane.id),
            enabled: lane.enabled,
            transport: ManagedTransport::Rathole,
            role: match lane.role {
                AgentLaneRole::Server => ManagedRole::Server,
                AgentLaneRole::Client => ManagedRole::Client,
            },
            control_port: lane.peer.port,
            listener: Listener {
                bind: lane.listener.bind.clone(),
                port: lane.listener.port,
                target: self.target.as_ref().map(|target| TcpTarget {
                    host: target.host.clone(),
                    port: target.port,
                }),
            },
            scheme: TunnelListenerScheme::Tcp,
            peer_host: lane.peer.host.clone(),
            local_key: self.local_key.clone(),
            peer_key: self.peer_key.clone(),
            generation: self.generation.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WstunnelLane {
    pub certificate_file: String,
    pub key_file: String,
    pub peer_ca_file: String,
    pub tls_name: Option<String>,
    pub target: Option<AgentTarget>,
}

impl WstunnelLane {
    pub(crate) fn validate(&self, lane: &AgentLane) -> Result<(), String> {
        validate_target(lane, self.target.as_ref())?;
        let valid_name = match lane.role {
            AgentLaneRole::Server => self.tls_name.is_none(),
            AgentLaneRole::Client => self.tls_name.as_deref().is_some_and(|name| {
                crate::managed_reverse::valid_host(name)
                    && name.parse::<std::net::IpAddr>().is_err()
            }),
        };
        if !valid_name
            || [&self.certificate_file, &self.key_file, &self.peer_ca_file]
                .into_iter()
                .any(|path| !Path::new(path).is_absolute() || path.chars().any(char::is_control))
        {
            return Err(format!(
                "agent lane {:?} requires valid mutual TLS settings",
                lane.id
            ));
        }
        Ok(())
    }

    fn compile(&self, lane: &AgentLane, bundle: &Path, state: &Path) -> LaneCompilation {
        let mut argv = vec![
            bundle.join("bin/wstunnel").display().to_string(),
            match lane.role {
                AgentLaneRole::Server => "server",
                AgentLaneRole::Client => "client",
            }
            .to_owned(),
            "--log-lvl".to_owned(),
            "WARN".to_owned(),
            "--nb-worker-threads".to_owned(),
            "1".to_owned(),
            "--tls-certificate".to_owned(),
            self.certificate_file.clone(),
            "--tls-private-key".to_owned(),
            self.key_file.clone(),
        ];
        let mut env = BTreeMap::from([("TOKIO_WORKER_THREADS".to_owned(), "1".to_owned())]);
        let mut files = Vec::new();
        let listeners = match lane.role {
            AgentLaneRole::Server => {
                files.push(self.server_args(lane, &mut argv, state));
                [
                    ("0.0.0.0", lane.peer.port),
                    ("127.0.0.1", lane.listener.port),
                ]
                .map(|(bind, port)| ListenerClaim {
                    owner: format!("agent lane {}", lane.id),
                    bind: bind.to_owned(),
                    port,
                })
                .into()
            }
            AgentLaneRole::Client => {
                self.client_args(lane, &mut argv);
                env.insert("SSL_CERT_FILE".to_owned(), self.peer_ca_file.clone());
                Vec::new()
            }
        };
        let process = ProcessSpec {
            id: format!("agent-lane-{}", lane.id),
            argv,
            env,
            workdir: None,
            restart: lane.restart.clone(),
        };
        (vec![process], files, listeners)
    }

    fn server_args(&self, lane: &AgentLane, argv: &mut Vec<String>, state: &Path) -> GeneratedFile {
        let path = state
            .join("generated")
            .join(format!("agent-lane-{}.yaml", lane.id));
        argv.extend([
            "--tls-client-ca-certs".to_owned(),
            self.peer_ca_file.clone(),
            "--restrict-config".to_owned(),
            path.display().to_string(),
            format!("wss://0.0.0.0:{}", lane.peer.port),
        ]);
        let content = format!(
            "restrictions:\n  - name: management\n    match:\n      - !Any\n    allow:\n      - !ReverseTunnel\n        protocol: [Tcp]\n        port: [{}]\n        cidr: [127.0.0.1/32]\n",
            lane.listener.port,
        );
        GeneratedFile { path, content }
    }

    fn client_args(&self, lane: &AgentLane, argv: &mut Vec<String>) {
        let target = self.target.as_ref().expect("validated WSTunnel target");
        argv.extend([
            "--tls-sni-override".to_owned(),
            self.tls_name.clone().expect("validated TLS name"),
            "--tls-verify-certificate".to_owned(),
            "--reverse-tunnel-connection-retry-max-backoff".to_owned(),
            "15s".to_owned(),
            "--remote-to-local".to_owned(),
            format!(
                "tcp://127.0.0.1:{}:{}",
                lane.listener.port,
                authority(&target.host, target.port)
            ),
            format!("wss://{}", authority(&lane.peer.host, lane.peer.port)),
        ]);
    }
}

fn validate_target(lane: &AgentLane, target: Option<&AgentTarget>) -> Result<(), String> {
    if lane.listener.bind != "127.0.0.1"
        || lane.listener.port == lane.peer.port
        || !match (lane.role, target) {
            (AgentLaneRole::Server, None) => true,
            (AgentLaneRole::Client, Some(target)) => {
                target.host == "127.0.0.1" && target.is_loopback()
            }
            _ => false,
        }
    {
        return Err(format!(
            "agent lane {:?} requires a dedicated IPv4 loopback mapping",
            lane.id
        ));
    }
    Ok(())
}

pub(crate) fn compile(
    lane: &AgentLane,
    bundle: &Path,
    state: &Path,
    check_only: bool,
) -> Result<LaneCompilation, String> {
    if let Some(wstunnel) = &lane.wstunnel {
        return Ok(wstunnel.compile(lane, bundle, state));
    }
    let plan = lane
        .rathole
        .as_ref()
        .expect("validated Rathole settings")
        .contract(lane)
        .compile(&bundle.join("bin/rathole"), bundle, state, check_only)?;
    let mut process = plan.process;
    process.restart = lane.restart.clone();
    process
        .env
        .insert("TOKIO_WORKER_THREADS".to_owned(), "1".to_owned());
    Ok((vec![process], plan.generated, plan.listeners))
}
