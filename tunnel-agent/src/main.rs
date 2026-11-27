mod agent;
mod agent_control;
mod agent_lane;
mod agent_poll;
mod agent_relays;
mod agent_spec;
mod agent_transports;
#[cfg(test)]
mod agent_transports_live_tests;
mod capsule;
mod foreign_update;
mod hpn_server_worker;
mod identity;
mod ingress;
mod managed_reverse;
mod manifest;
mod probe_job;
mod relay;
mod release;
mod runtime;
mod seed;
mod seed_client;
mod seed_relay;
mod seed_server;
mod telemetry;
mod transport_identity;
mod tunnel_probes;

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::ExitCode,
};

use agent::AgentSettings;
use clap::{Parser, Subcommand};
use manifest::CompiledPlan;

const VERSION: &str = match option_env!("TUNNEL_AGENT_VERSION") {
    Some(version) => version,
    None => concat!("v", env!("CARGO_PKG_VERSION")),
};

#[derive(Parser)]
#[command(name = "tunnel-agent", version = VERSION, about = "Low-footprint multi-transport tunnel runtime")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    #[command(hide = true)]
    ProbeJobWorker {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
    #[command(hide = true)]
    PublicIngressWorker {
        #[arg(long)]
        config: PathBuf,
    },
    #[command(hide = true)]
    HpnServerWorker {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        config: PathBuf,
    },
    /// Validate a v1, v2, or pre-bundled v3 tunnel manifest without starting anything.
    Validate {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/tunnel-agent")]
        state_dir: PathBuf,
    },
    /// Print the process and listener plan without starting anything.
    Plan {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/tunnel-agent")]
        state_dir: PathBuf,
    },
    /// Download, verify, and atomically install pinned v2 artifacts. v3 bundles are preinstalled.
    Sync {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/tunnel-agent")]
        state_dir: PathBuf,
        #[arg(long)]
        artifact: Option<String>,
    },
    /// Supervise every compiled process until SIGINT or SIGTERM.
    Run {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/tunnel-agent")]
        state_dir: PathBuf,
    },
    /// Reconcile an authenticated v3 document and supervise its typed contracts.
    Agent {
        #[arg(long, default_value = "/etc/tunnel-agent/manifest.json")]
        config: PathBuf,
        #[arg(long, default_value = "/opt/tunnel-agent/state")]
        state_dir: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/api-url")]
        api_file: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/node-id")]
        node_id_file: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/node-token")]
        node_token_file: PathBuf,
        /// Override agent.poll_interval_secs from the manifest for this process.
        #[arg(long)]
        poll_interval_secs: Option<u64>,
        /// Use the signed domestic bootstrap state only; never call the API.
        #[arg(long)]
        offline: bool,
    },
    /// Verify and persist a signed domestic bootstrap capsule.
    BootstrapCapsule {
        #[arg(long)]
        capsule: String,
        #[arg(long, default_value = "/etc/tunnel-agent")]
        etc_dir: PathBuf,
        #[arg(long, default_value = "/opt/tunnel-agent/state")]
        state_dir: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/manifest.json")]
        config: PathBuf,
    },
    /// Verify a signed relay document for this domestic node without applying it.
    VerifyRelayDocument {
        /// Read the envelope from this file instead of standard input.
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long, default_value = "/etc/tunnel-agent/relay-state.json")]
        relay_state: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/node-id")]
        node_id_file: PathBuf,
    },
    /// Cache the signed active release document for a foreign seed node.
    CacheReleaseDocument {
        #[arg(long, default_value = "/etc/tunnel-agent/api-url")]
        api_file: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/node-id")]
        node_id_file: PathBuf,
        #[arg(long, default_value = "/etc/tunnel-agent/node-token")]
        node_token_file: PathBuf,
        #[arg(long, default_value = "/opt/tunnel-agent/state/release-document.json")]
        output: PathBuf,
    },
    /// Serve the current verified bundle over an agent-provided loopback lane.
    SeedServe {
        #[arg(long, default_value = "/opt/tunnel-agent")]
        root: PathBuf,
        #[arg(long)]
        listen: std::net::SocketAddr,
        #[arg(long, default_value = "/opt/tunnel-agent/state/release-document.json")]
        release_document: PathBuf,
        /// Directory containing API-signed domestic relay envelopes.
        #[arg(long)]
        relay_directory: Option<PathBuf>,
    },
    /// Fetch a verified bundle from a loopback agent lane and stage or activate it.
    SeedApply {
        #[arg(long, default_value = "/opt/tunnel-agent")]
        root: PathBuf,
        #[arg(long)]
        source: String,
        /// Atomically advance the current release symlink after verification.
        #[arg(long)]
        activate: bool,
        /// Restart this systemd service after --activate succeeds.
        #[arg(long)]
        restart_service: Option<String>,
    },
}

fn print_plan(plan: &CompiledPlan) -> Result<(), String> {
    let document = serde_json::json!({
        "processes": plan.processes.iter().map(|process| process.id.as_str()).collect::<Vec<_>>(),
        "local_listeners": plan.listeners.iter()
            .map(|listener| format!("{}:{} ({})", listener.bind, listener.port, listener.owner))
            .collect::<Vec<_>>(),
        "generated_files": plan.generated_files.iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>(),
        "agent_lanes": plan.agent_lanes.iter().map(|lane| serde_json::json!({
            "id": lane.id,
            "selected": lane.selected,
            "transport": lane.transport.as_str(),
            "direction": lane.direction.as_str(),
            "role": format!("{:?}", lane.role).to_lowercase(),
            "priority": lane.priority,
            "peer_node_id": lane.peer.node_id,
            "peer": format!("{}:{}", lane.peer.host, lane.peer.port),
            "listener": format!("{}:{}", lane.listener.bind, lane.listener.port),
        })).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn compiled_plan(config: &Path, state_dir: &Path) -> Result<CompiledPlan, String> {
    manifest::load_config(config).and_then(|config| manifest::compile(&config, state_dir))
}

fn main() -> ExitCode {
    match Cli::parse().command {
        // Blocking reqwest can exhaust the current-thread runtime's cooperative budget.
        CommandKind::PublicIngressWorker { config } => ingress::run(&config).map_or_else(
            |error| failure("public ingress worker failed", error),
            |()| ExitCode::SUCCESS,
        ),
        command => execute_async(command),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn execute_async(command: CommandKind) -> ExitCode {
    match command {
        CommandKind::ProbeJobWorker { config, state } => probe_job::run(&config, &state)
            .map_or_else(
                |error| failure("probe job worker failed", error),
                |()| ExitCode::SUCCESS,
            ),
        CommandKind::PublicIngressWorker { .. } => unreachable!("dispatched synchronously"),
        CommandKind::HpnServerWorker { binary, config } => {
            hpn_server_worker::run(&binary, &config).await.map_or_else(
                |error| failure("HPNSSH worker failed", error),
                |()| ExitCode::SUCCESS,
            )
        }
        CommandKind::Validate { config, state_dir } => validate_command(&config, &state_dir),
        CommandKind::Plan { config, state_dir } => plan_command(&config, &state_dir),
        CommandKind::Sync {
            config,
            state_dir,
            artifact,
        } => sync_command(&config, &state_dir, artifact.as_deref()),
        CommandKind::Run { config, state_dir } => run_command(&config, &state_dir).await,
        CommandKind::Agent {
            config,
            state_dir,
            api_file,
            node_id_file,
            node_token_file,
            poll_interval_secs,
            offline,
        } => {
            agent_command(
                config,
                state_dir,
                api_file,
                node_id_file,
                node_token_file,
                poll_interval_secs,
                offline,
            )
            .await
        }
        CommandKind::BootstrapCapsule {
            capsule,
            etc_dir,
            state_dir,
            config,
        } => bootstrap_command(&capsule, &etc_dir, &state_dir, &config),
        CommandKind::VerifyRelayDocument {
            input,
            relay_state,
            node_id_file,
        } => verify_relay_command(input.as_deref(), &relay_state, &node_id_file),
        CommandKind::CacheReleaseDocument {
            api_file,
            node_id_file,
            node_token_file,
            output,
        } => cache_release_command(&api_file, &node_id_file, &node_token_file, &output),
        CommandKind::SeedServe {
            root,
            listen,
            release_document,
            relay_directory,
        } => seed::serve(&root, listen, &release_document, relay_directory.as_deref()).map_or_else(
            |error| failure("seed server cannot start", error),
            |_| ExitCode::SUCCESS,
        ),
        CommandKind::SeedApply {
            root,
            source,
            activate,
            restart_service,
        } => seed_apply_command(&root, &source, activate, restart_service.as_deref()),
    }
}

fn validate_command(config: &Path, state_dir: &Path) -> ExitCode {
    match manifest::load_config(config)
        .and_then(|config| manifest::validate_plan(&config, state_dir).map(|plan| (config, plan)))
    {
        Ok((config, plan)) => {
            println!(
                "valid version={} processes={} tunnels={} agent_lanes={} listeners={}",
                config.version,
                plan.processes.len(),
                config.tunnels.len(),
                plan.agent_lanes.len(),
                plan.listeners.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => failure("invalid", error),
    }
}

fn plan_command(config: &Path, state_dir: &Path) -> ExitCode {
    compiled_plan(config, state_dir)
        .and_then(|plan| print_plan(&plan))
        .map_or_else(
            |error| failure("cannot compile plan", error),
            |_| ExitCode::SUCCESS,
        )
}

fn sync_command(config: &Path, state_dir: &Path, artifact: Option<&str>) -> ExitCode {
    match manifest::load_config(config)
        .and_then(|config| manifest::sync_artifacts(&config, state_dir, artifact))
    {
        Ok(paths) => {
            for path in paths {
                println!("synchronized {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure("cannot synchronize artifacts", error),
    }
}

async fn run_command(config: &Path, state_dir: &Path) -> ExitCode {
    match compiled_plan(config, state_dir) {
        Ok(plan) => {
            eprintln!(
                "tunnel-agent version={VERSION} processes={}",
                plan.processes.len()
            );
            runtime::run(plan).await.map_or_else(
                |error| failure("cannot start", error),
                |_| ExitCode::SUCCESS,
            )
        }
        Err(error) => failure("cannot start", error),
    }
}

async fn agent_command(
    config: PathBuf,
    state_dir: PathBuf,
    api_file: PathBuf,
    node_id_file: PathBuf,
    node_token_file: PathBuf,
    poll_interval_secs: Option<u64>,
    offline: bool,
) -> ExitCode {
    agent::reconcile(AgentSettings {
        config_path: config,
        state_dir,
        api_file,
        node_id_file,
        node_token_file,
        poll_interval_secs,
        offline,
    })
    .await
    .map_or_else(
        |error| failure("agent cannot start", error),
        |_| ExitCode::SUCCESS,
    )
}

fn bootstrap_command(
    capsule_value: &str,
    etc_dir: &Path,
    state_dir: &Path,
    config: &Path,
) -> ExitCode {
    match capsule::install_domestic(capsule_value, etc_dir, state_dir, config) {
        Ok(material) => {
            println!(
                "installed domestic capsule cluster={} node={} revision={}",
                material.cluster_id, material.node_id, material.revision
            );
            ExitCode::SUCCESS
        }
        Err(error) => failure("invalid bootstrap capsule", error),
    }
}

fn verify_relay_command(input: Option<&Path>, relay_state: &Path, node_id_file: &Path) -> ExitCode {
    match read_relay_document(input)
        .and_then(|raw| relay::verify_for_local_node(&raw, relay_state, node_id_file))
    {
        Ok(document) => {
            println!(
                "verified relay document cluster={} node={} revision={}",
                document.cluster_id, document.node_id, document.revision
            );
            ExitCode::SUCCESS
        }
        Err(error) => failure("invalid relay document", error),
    }
}

fn cache_release_command(
    api_file: &Path,
    node_id_file: &Path,
    node_token_file: &Path,
    output: &Path,
) -> ExitCode {
    let result = read_text(api_file, "API URL").and_then(|api_url| {
        read_text(node_id_file, "node id").and_then(|node_id| {
            read_text(node_token_file, "node token").and_then(|node_token| {
                release::cache_release_document(&api_url, &node_id, &node_token, output)
            })
        })
    });
    result.map_or_else(
        |error| failure("cannot cache signed release document", error),
        |_| {
            println!("cached signed release document at {}", output.display());
            ExitCode::SUCCESS
        },
    )
}

fn seed_apply_command(
    root: &Path,
    source: &str,
    activate: bool,
    restart_service: Option<&str>,
) -> ExitCode {
    if restart_service.is_some() && !activate {
        return failure(
            "cannot apply seeded release",
            "--restart-service requires --activate".to_owned(),
        );
    }
    let result = seed::apply(root, source, activate).and_then(|result| {
        if let Some(service) = restart_service {
            seed::restart_service(service)?;
        }
        Ok(result)
    });
    result.map_or_else(
        |error| failure("cannot apply seeded release", error),
        |result| {
            println!(
                "seeded version={} downloaded_files={} reused_files={} changed={} activated={}",
                result.version,
                result.downloaded_files,
                result.reused_files,
                result.changed,
                result.activated
            );
            ExitCode::SUCCESS
        },
    )
}

fn read_relay_document(input: Option<&Path>) -> Result<String, String> {
    match input {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read relay document {}: {error}", path.display())),
        None => {
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .map_err(|error| {
                    format!("cannot read relay document from standard input: {error}")
                })?;
            Ok(value)
        }
    }
}

fn read_text(path: &Path, label: &str) -> Result<String, String> {
    let value = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(format!("{label} {} is empty", path.display()))
    } else {
        Ok(value)
    }
}

fn failure(context: &str, error: String) -> ExitCode {
    eprintln!("{context}: {error}");
    ExitCode::FAILURE
}
