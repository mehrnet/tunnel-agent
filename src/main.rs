use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::PathBuf,
    process::{ExitCode, Stdio},
    time::Duration,
};

use clap::{Parser, Subcommand};
use serde::Deserialize;
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
#[command(name = "tunnel-agent", version = VERSION, about = "Low-footprint tunnel process supervisor")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    /// Validate a process manifest without starting anything.
    Validate {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
    },
    /// Supervise every process in the manifest until SIGINT or SIGTERM.
    Run {
        #[arg(long, default_value = "/etc/tunnel-agent.json")]
        config: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    processes: Vec<ProcessSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessSpec {
    id: String,
    argv: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    workdir: Option<PathBuf>,
    #[serde(default)]
    restart: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartPolicy {
    #[serde(default = "default_initial_backoff_ms")]
    initial_backoff_ms: u64,
    #[serde(default = "default_max_backoff_ms")]
    max_backoff_ms: u64,
    #[serde(default = "default_reset_after_secs")]
    reset_after_secs: u64,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: default_initial_backoff_ms(),
            max_backoff_ms: default_max_backoff_ms(),
            reset_after_secs: default_reset_after_secs(),
        }
    }
}

fn default_initial_backoff_ms() -> u64 {
    1_000
}

fn default_max_backoff_ms() -> u64 {
    60_000
}

fn default_reset_after_secs() -> u64 {
    60
}

fn load_config(path: &PathBuf) -> Result<Config, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let config: Config = serde_json::from_str(&raw)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &Config) -> Result<(), String> {
    if config.version != 1 {
        return Err(format!(
            "unsupported manifest version {}; expected 1",
            config.version
        ));
    }
    if config.processes.is_empty() {
        return Err("processes must not be empty".to_owned());
    }

    let mut ids = HashSet::new();
    for process in &config.processes {
        if process.id.trim().is_empty() {
            return Err("process id must not be empty".to_owned());
        }
        if !ids.insert(process.id.as_str()) {
            return Err(format!("duplicate process id {:?}", process.id));
        }
        if process.argv.is_empty() || process.argv[0].trim().is_empty() {
            return Err(format!(
                "process {:?} must contain a non-empty argv[0]",
                process.id
            ));
        }
        if process.restart.initial_backoff_ms == 0 {
            return Err(format!(
                "process {:?} initial_backoff_ms must be positive",
                process.id
            ));
        }
        if process.restart.max_backoff_ms < process.restart.initial_backoff_ms {
            return Err(format!(
                "process {:?} max_backoff_ms must be at least initial_backoff_ms",
                process.id
            ));
        }
        if process.restart.reset_after_secs == 0 {
            return Err(format!(
                "process {:?} reset_after_secs must be positive",
                process.id
            ));
        }
        if process
            .env
            .keys()
            .any(|key| key.is_empty() || key.contains('='))
        {
            return Err(format!(
                "process {:?} has an invalid environment key",
                process.id
            ));
        }
    }
    Ok(())
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
        // Child processes are protocol binaries. Give their own cleanup code a chance
        // before falling back to SIGKILL, matching systemd's normal stop behaviour.
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
                    changed = shutdown.changed() => {
                        if changed.is_ok() && *shutdown.borrow() { return; }
                    }
                }
                backoff = (backoff.saturating_mul(2)).min(max_backoff);
                continue;
            }
        };

        let pid = child.id().unwrap_or_default();
        eprintln!("process={} pid={} started", spec.id, pid);
        let started = Instant::now();
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(status) => eprintln!("process={} pid={} exited status={status}", spec.id, pid),
                    Err(error) => eprintln!("process={} pid={} wait failed: {error}", spec.id, pid),
                }
            }
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
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() { return; }
            }
        }
        backoff = (backoff.saturating_mul(2)).min(max_backoff);
    }
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).expect("cannot install SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("cannot install SIGINT handler");
    tokio::select! {
        _ = terminate.recv() => {},
        _ = interrupt.recv() => {},
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn run(config: Config) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut handles = Vec::with_capacity(config.processes.len());
    for process in config.processes {
        handles.push(tokio::spawn(supervise(process, shutdown_rx.clone())));
    }

    wait_for_shutdown().await;
    eprintln!("shutdown signal received");
    let _ = shutdown_tx.send(true);
    for handle in handles {
        let _ = handle.await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Validate { config } => match load_config(&config) {
            Ok(config) => {
                println!(
                    "valid version={} processes={}",
                    config.version,
                    config.processes.len()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("invalid: {error}");
                ExitCode::FAILURE
            }
        },
        CommandKind::Run { config } => match load_config(&config) {
            Ok(config) => {
                eprintln!(
                    "tunnel-agent version={VERSION} processes={}",
                    config.processes.len()
                );
                run(config).await;
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("cannot start: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(processes: Vec<ProcessSpec>) -> Config {
        Config {
            version: 1,
            processes,
        }
    }

    fn process(id: &str) -> ProcessSpec {
        ProcessSpec {
            id: id.to_owned(),
            argv: vec!["/bin/true".to_owned()],
            env: BTreeMap::new(),
            workdir: None,
            restart: RestartPolicy::default(),
        }
    }

    #[test]
    fn accepts_a_minimal_process() {
        assert!(validate_config(&config(vec![process("first")])).is_ok());
    }

    #[test]
    fn rejects_duplicate_process_ids() {
        assert!(validate_config(&config(vec![process("same"), process("same")])).is_err());
    }

    #[test]
    fn rejects_invalid_backoff_range() {
        let mut spec = process("first");
        spec.restart.initial_backoff_ms = 10;
        spec.restart.max_backoff_ms = 9;
        assert!(validate_config(&config(vec![spec])).is_err());
    }
}
