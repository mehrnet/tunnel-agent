mod agent;
mod manifest;
mod runtime;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use agent::AgentSettings;
use clap::{Parser, Subcommand};
use manifest::CompiledPlan;

const VERSION: &str = match option_env!("TUNNEL_AGENT_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(name = "tunnel-agent", version = VERSION, about = "Low-footprint multi-transport tunnel runtime")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
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
        #[arg(long, default_value_t = 60)]
        poll_interval_secs: u64,
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match Cli::parse().command {
        CommandKind::Validate { config, state_dir } => match manifest::load_config(&config)
            .and_then(|config| manifest::compile(&config, &state_dir).map(|plan| (config, plan)))
        {
            Ok((config, plan)) => {
                println!(
                    "valid version={} processes={} tunnels={} listeners={}",
                    config.version,
                    plan.processes.len(),
                    config.tunnels.len(),
                    plan.listeners.len()
                );
                ExitCode::SUCCESS
            }
            Err(error) => failure("invalid", error),
        },
        CommandKind::Plan { config, state_dir } => match compiled_plan(&config, &state_dir)
            .and_then(|plan| print_plan(&plan).map(|()| plan))
        {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => failure("cannot compile plan", error),
        },
        CommandKind::Sync {
            config,
            state_dir,
            artifact,
        } => match manifest::load_config(&config)
            .and_then(|config| manifest::sync_artifacts(&config, &state_dir, artifact.as_deref()))
        {
            Ok(paths) => {
                for path in paths {
                    println!("synchronized {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure("cannot synchronize artifacts", error),
        },
        CommandKind::Run { config, state_dir } => match compiled_plan(&config, &state_dir) {
            Ok(plan) => {
                eprintln!(
                    "tunnel-agent version={VERSION} processes={}",
                    plan.processes.len()
                );
                match runtime::run(plan).await {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => failure("cannot start", error),
                }
            }
            Err(error) => failure("cannot start", error),
        },
        CommandKind::Agent {
            config,
            state_dir,
            api_file,
            node_id_file,
            node_token_file,
            poll_interval_secs,
        } => match agent::reconcile(AgentSettings {
            config_path: config,
            state_dir,
            api_file,
            node_id_file,
            node_token_file,
            poll_interval_secs,
        })
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => failure("agent cannot start", error),
        },
    }
}

fn failure(context: &str, error: String) -> ExitCode {
    eprintln!("{context}: {error}");
    ExitCode::FAILURE
}
