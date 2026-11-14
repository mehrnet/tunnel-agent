mod manifest;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
    time::Duration,
};

use clap::{Parser, Subcommand};
use manifest::{CompiledPlan, Config, ProcessSpec};
use reqwest::{
    StatusCode,
    blocking::Client as BlockingClient,
    header::{ETAG, IF_NONE_MATCH},
};
use serde::Deserialize;
use tokio::{
    process::{Child, Command},
    sync::watch,
    time::{Instant, MissedTickBehavior, interval, sleep, timeout},
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

struct RunningPlan {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

fn spawn_plan(plan: CompiledPlan) -> RunningPlan {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut handles = Vec::with_capacity(plan.processes.len());
    for process in plan.processes {
        handles.push(tokio::spawn(supervise(process, shutdown_rx.clone())));
    }
    RunningPlan {
        shutdown_tx,
        handles,
    }
}

async fn start_plan(plan: CompiledPlan) -> Result<RunningPlan, String> {
    manifest::write_generated_files(&plan)?;
    Ok(spawn_plan(plan))
}

async fn stop_plan(plan: &mut RunningPlan) {
    let _ = plan.shutdown_tx.send(true);
    for handle in plan.handles.drain(..) {
        let _ = handle.await;
    }
}

async fn run(plan: CompiledPlan) -> Result<(), String> {
    let mut running = start_plan(plan).await?;
    wait_for_shutdown().await;
    eprintln!("shutdown signal received");
    stop_plan(&mut running).await;
    Ok(())
}

const MAX_DOCUMENT_BYTES: usize = 1_048_576;

#[derive(Debug, Deserialize)]
struct RemoteDocument {
    revision: u64,
    manifest_json: String,
}

enum DocumentFetch {
    Unchanged,
    Updated {
        document: RemoteDocument,
        etag: String,
    },
}

fn read_secret_file(path: &Path, label: &str) -> Result<String, String> {
    let value = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(format!("{label} {} is empty", path.display()));
    }
    Ok(value)
}

fn validate_node_credentials(node_id: &str, token: &str) -> Result<(), String> {
    if node_id.len() != 36
        || !node_id.chars().enumerate().all(|(index, character)| {
            (matches!(index, 8 | 13 | 18 | 23) && character == '-') || character.is_ascii_hexdigit()
        })
    {
        return Err("node id must be a UUID".to_owned());
    }
    if !token.starts_with(&format!("{node_id}.")) || token.len() <= node_id.len() + 1 {
        return Err("node token does not belong to the configured node id".to_owned());
    }
    Ok(())
}

fn read_revision(path: &Path) -> Result<u64, String> {
    match fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("cannot parse revision {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("cannot read revision {}: {error}", path.display())),
    }
}

fn ensure_compatible_document(current: &Config, candidate: &Config) -> Result<(), String> {
    if current.version != 3 || candidate.version != 3 {
        return Err("agent accepts version 3 manifests only".to_owned());
    }
    if current.node_scope != candidate.node_scope {
        return Err("remote document cannot change node_scope".to_owned());
    }
    let current_root = current
        .bundle
        .as_ref()
        .ok_or_else(|| "active manifest has no bundle root".to_owned())?
        .root
        .as_path();
    let candidate_root = candidate
        .bundle
        .as_ref()
        .ok_or_else(|| "remote document has no bundle root".to_owned())?
        .root
        .as_path();
    if current_root != candidate_root {
        return Err("remote document cannot change bundle.root".to_owned());
    }
    Ok(())
}

fn fetch_document(
    client: &BlockingClient,
    api_url: &str,
    node_id: &str,
    token: &str,
    etag: Option<&str>,
) -> Result<DocumentFetch, String> {
    let endpoint = format!(
        "{}/v1/nodes/{node_id}/document",
        api_url.trim_end_matches('/')
    );
    let mut request = client.get(endpoint).bearer_auth(token);
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .map_err(|error| format!("cannot fetch desired document: {error}"))?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(DocumentFetch::Unchanged);
    }
    if !response.status().is_success() {
        return Err(format!(
            "desired document request returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64)
    {
        return Err("desired document exceeds maximum size".to_owned());
    }
    let response_etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .text()
        .map_err(|error| format!("cannot read desired document: {error}"))?;
    if body.len() > MAX_DOCUMENT_BYTES {
        return Err("desired document exceeds maximum size".to_owned());
    }
    let document: RemoteDocument = serde_json::from_str(&body)
        .map_err(|error| format!("cannot parse desired document: {error}"))?;
    Ok(DocumentFetch::Updated {
        etag: response_etag.unwrap_or_else(|| format!("\"{}\"", document.revision)),
        document,
    })
}

async fn reconcile(
    config_path: PathBuf,
    state_dir: PathBuf,
    api_file: PathBuf,
    node_id_file: PathBuf,
    node_token_file: PathBuf,
    poll_interval_secs: u64,
) -> Result<(), String> {
    if poll_interval_secs < 5 {
        return Err("poll_interval_secs must be at least 5".to_owned());
    }

    let api_url = read_secret_file(&api_file, "API URL")?;
    if !api_url.starts_with("https://") {
        return Err("API URL must use HTTPS".to_owned());
    }
    let node_id = read_secret_file(&node_id_file, "node id")?;
    let node_token = read_secret_file(&node_token_file, "node token")?;
    validate_node_credentials(&node_id, &node_token)?;

    let mut active_config = manifest::load_config(&config_path)?;
    if active_config.version != 3 {
        return Err("agent requires a version 3 bootstrap manifest".to_owned());
    }
    let initial_plan = manifest::compile(&active_config, &state_dir)?;
    let mut running = start_plan(initial_plan).await?;
    fs::create_dir_all(&state_dir)
        .map_err(|error| format!("cannot create {}: {error}", state_dir.display()))?;
    let revision_path = state_dir.join("document-revision");
    let mut active_revision = read_revision(&revision_path)?;
    let mut etag = if active_revision == 0 {
        None
    } else {
        Some(format!("\"{active_revision}\""))
    };
    let client = BlockingClient::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("cannot build API client: {error}"))?;
    let mut poll = interval(Duration::from_secs(poll_interval_secs));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = wait_for_shutdown();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("shutdown signal received");
                stop_plan(&mut running).await;
                return Ok(());
            }
            _ = poll.tick() => {
                let client = client.clone();
                let api_url = api_url.clone();
                let node_id = node_id.clone();
                let node_token = node_token.clone();
                let request_etag = etag.clone();
                let response = tokio::task::spawn_blocking(move || {
                    fetch_document(&client, &api_url, &node_id, &node_token, request_etag.as_deref())
                }).await;
                match response {
                    Ok(Ok(DocumentFetch::Unchanged)) => {},
                    Ok(Ok(DocumentFetch::Updated { document, etag: new_etag })) => {
                        if document.revision <= active_revision {
                            etag = Some(new_etag);
                            continue;
                        }
                        let (candidate_config, candidate_plan) = match manifest::parse_config(&document.manifest_json)
                            .and_then(|config| {
                                ensure_compatible_document(&active_config, &config)?;
                                manifest::compile(&config, &state_dir).map(|plan| (config, plan))
                            }) {
                            Ok(candidate) => candidate,
                            Err(error) => {
                                eprintln!(
                                    "document revision={} rejected; retaining revision={active_revision}: {error}",
                                    document.revision
                                );
                                continue;
                            }
                        };
                        if let Err(error) = manifest::write_generated_files(&candidate_plan)
                            .and_then(|()| manifest::write_config_atomic(&config_path, &document.manifest_json)) {
                            eprintln!(
                                "document revision={} could not be staged; retaining revision={active_revision}: {error}",
                                document.revision
                            );
                            continue;
                        }
                        eprintln!(
                            "applying document revision={} replacing revision={active_revision}",
                            document.revision
                        );
                        stop_plan(&mut running).await;
                        running = spawn_plan(candidate_plan);
                        manifest::write_config_atomic(&revision_path, &document.revision.to_string())?;
                        active_config = candidate_config;
                        active_revision = document.revision;
                        etag = Some(new_etag);
                    }
                    Ok(Err(error)) => eprintln!(
                        "document refresh failed; retaining revision={active_revision}: {error}"
                    ),
                    Err(error) => eprintln!(
                        "document refresh task failed; retaining revision={active_revision}: {error}"
                    ),
                }
            }
        }
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
            Ok(plan) => {
                eprintln!(
                    "tunnel-agent version={VERSION} processes={}",
                    plan.processes.len()
                );
                match run(plan).await {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => {
                        eprintln!("cannot start: {error}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(error) => {
                eprintln!("cannot start: {error}");
                ExitCode::FAILURE
            }
        },
        CommandKind::Agent {
            config,
            state_dir,
            api_file,
            node_id_file,
            node_token_file,
            poll_interval_secs,
        } => match reconcile(
            config,
            state_dir,
            api_file,
            node_id_file,
            node_token_file,
            poll_interval_secs,
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("agent cannot start: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    const NODE_ID: &str = "d6caf939-b959-4471-918f-2e05ca282a37";

    fn bundled_config(scope: &str, root: &str) -> Config {
        manifest::parse_config(&format!(
            r#"{{
                "version": 3,
                "node_scope": "{scope}",
                "bundle": {{ "root": "{root}" }},
                "tunnels": []
            }}"#
        ))
        .expect("valid v3 test manifest")
    }

    #[test]
    fn accepts_node_token_bound_to_its_node_id() {
        validate_node_credentials(
            NODE_ID,
            "d6caf939-b959-4471-918f-2e05ca282a37.0123456789abcdef",
        )
        .expect("token is bound to node id");
    }

    #[test]
    fn rejects_node_token_for_another_node() {
        assert!(
            validate_node_credentials(
                NODE_ID,
                "43d4d9e2-9131-4e84-bcd5-f2baac3f0214.0123456789abcdef",
            )
            .is_err()
        );
    }

    #[test]
    fn remote_document_cannot_change_scope_or_bundle_root() {
        let active = bundled_config("domestic", "/opt/tunnel-agent/current");
        let changed_scope = bundled_config("foreign", "/opt/tunnel-agent/current");
        let changed_root = bundled_config("domestic", "/opt/tunnel-agent/other");
        assert!(ensure_compatible_document(&active, &changed_scope).is_err());
        assert!(ensure_compatible_document(&active, &changed_root).is_err());
    }
}
