mod manifest;

use std::{
    path::PathBuf,
    process::{ExitCode, Stdio},
    time::Duration,
};

use clap::{Parser, Subcommand};
use manifest::{CompiledPlan, ProcessSpec};
use tokio::{
    process::{Child, Command},
    sync::watch,
    time::{Instant, sleep, timeout},
};

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
    /// Validate a v1 legacy or v2 typed tunnel manifest without starting anything.
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
    /// Download, verify, and atomically install pinned artifacts. Never runs periodically.
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
}

fn spawn_child(spec: &ProcessSpec) -> std::io::Result<Child> {
    let mut command = Command::new(&spec.argv[0]);
    command
        .args(&spec.argv[1..])
        .envs(&spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    if let Some(workdir) = &spec.workdir {
        command.current_dir(workdir);
    }
    command.spawn()
}

async fn stop_child(child: &mut Child, id: &str) {
    if let Some(pid) = child.id() {
        // Give protocol binaries time to close control connections before SIGKILL.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    if timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        eprintln!("process={id} did not exit after SIGTERM; sending SIGKILL");
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

async fn supervise(spec: ProcessSpec, mut shutdown: watch::Receiver<bool>) {
    let initial_backoff = Duration::from_millis(spec.restart.initial_backoff_ms);
    let max_backoff = Duration::from_millis(spec.restart.max_backoff_ms);
    let reset_after = Duration::from_secs(spec.restart.reset_after_secs);
    let mut backoff = initial_backoff;

    loop {
        if *shutdown.borrow() {
            return;
        }
        let mut child = match spawn_child(&spec) {
            Ok(child) => child,
            Err(error) => {
                eprintln!(
                    "process={} spawn failed: {error}; retrying in {:?}",
                    spec.id, backoff
                );
                tokio::select! {
                    _ = sleep(backoff) => {},
                    changed = shutdown.changed() => if changed.is_ok() && *shutdown.borrow() { return; },
                }
                backoff = (backoff.saturating_mul(2)).min(max_backoff);
                continue;
            }
        };

        let pid = child.id().unwrap_or_default();
        eprintln!("process={} pid={} started", spec.id, pid);
        let started = Instant::now();
        tokio::select! {
            status = child.wait() => match status {
                Ok(status) => eprintln!("process={} pid={} exited status={status}", spec.id, pid),
                Err(error) => eprintln!("process={} pid={} wait failed: {error}", spec.id, pid),
            },
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    stop_child(&mut child, &spec.id).await;
                    return;
                }
            }
        }
        if started.elapsed() >= reset_after {
            backoff = initial_backoff;
        }
        eprintln!("process={} retrying in {:?}", spec.id, backoff);
        tokio::select! {
            _ = sleep(backoff) => {},
            changed = shutdown.changed() => if changed.is_ok() && *shutdown.borrow() { return; },
        }
        backoff = (backoff.saturating_mul(2)).min(max_backoff);
    }
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("cannot install SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("cannot install SIGINT handler");
    tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn run(plan: CompiledPlan) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut handles = Vec::with_capacity(plan.processes.len());
    for process in plan.processes {
        handles.push(tokio::spawn(supervise(process, shutdown_rx.clone())));
    }
    wait_for_shutdown().await;
    eprintln!("shutdown signal received");
    let _ = shutdown_tx.send(true);
    for handle in handles {
        let _ = handle.await;
    }
}

fn print_plan(plan: &CompiledPlan) -> Result<(), String> {
    let process_ids = plan
        .processes
        .iter()
        .map(|process| process.id.as_str())
        .collect::<Vec<_>>();
    let listeners = plan
        .listeners
        .iter()
        .map(|listener| format!("{}:{} ({})", listener.bind, listener.port, listener.owner))
        .collect::<Vec<_>>();
    let generated_files = plan
        .generated_files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "processes": process_ids,
        "local_listeners": listeners,
        "generated_files": generated_files,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
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
            Err(error) => {
                eprintln!("invalid: {error}");
                ExitCode::FAILURE
            }
        },
        CommandKind::Plan { config, state_dir } => match manifest::load_config(&config)
            .and_then(|config| manifest::compile(&config, &state_dir))
        {
            Ok(plan) => match print_plan(&plan) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("cannot print plan: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("cannot compile plan: {error}");
                ExitCode::FAILURE
            }
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
            Err(error) => {
                eprintln!("cannot synchronize artifacts: {error}");
                ExitCode::FAILURE
            }
        },
        CommandKind::Run { config, state_dir } => match manifest::load_config(&config)
            .and_then(|config| manifest::compile(&config, &state_dir))
        {
            Ok(plan) => match manifest::write_generated_files(&plan) {
                Ok(()) => {
                    eprintln!(
                        "tunnel-agent version={VERSION} processes={}",
                        plan.processes.len()
                    );
                    run(plan).await;
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("cannot write generated files: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("cannot start: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
