use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{
    process::{Child, Command},
    sync::watch,
    time::{Instant, sleep, timeout},
};

use crate::manifest::{self, CompiledPlan, GeneratedFile, ProcessSpec};

fn spawn_child(spec: &ProcessSpec) -> std::io::Result<Child> {
    let mut command = Command::new(&spec.argv[0]);
    command
        .args(&spec.argv[1..])
        .envs(&spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(false);
    if let Some(workdir) = &spec.workdir {
        command.current_dir(workdir);
    }
    #[cfg(test)]
    if std::env::var_os("TUNNEL_TEST_WORKER_LOGS").is_some() {
        command.stderr(Stdio::inherit());
    }
    command.spawn()
}

async fn stop_child(child: &mut Child, id: &str) {
    let pid = child.id();
    // Forwarding listeners may be held by a protocol worker's forked sessions.
    signal_worker_group(pid, libc::SIGTERM);
    if timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        eprintln!("process={id} did not exit after SIGTERM; sending SIGKILL");
        signal_worker_group(pid, libc::SIGKILL);
        let _ = child.wait().await;
    }
    signal_worker_group(pid, libc::SIGKILL);
}

fn signal_worker_group(pid: Option<u32>, signal: libc::c_int) {
    if let Some(group) = pid
        .and_then(|pid| libc::pid_t::try_from(pid).ok())
        .filter(|pid| *pid > 0)
    {
        unsafe { libc::kill(-group, signal) };
    }
}

async fn supervise(spec: ProcessSpec, mut shutdown: watch::Receiver<bool>) {
    let initial_backoff = Duration::from_millis(spec.restart.initial_backoff_ms);
    let max_backoff = Duration::from_millis(spec.restart.max_backoff_ms);
    let reset_after = Duration::from_secs(spec.restart.reset_after_secs);
    let mut backoff = initial_backoff;

    loop {
        if stopping(&shutdown) {
            return;
        }
        let mut child = match spawn_child(&spec) {
            Ok(child) => child,
            Err(error) => {
                eprintln!(
                    "process={} spawn failed: {error}; retrying in {:?}",
                    spec.id, backoff
                );
                wait_or_shutdown(backoff, &mut shutdown).await;
                if stopping(&shutdown) {
                    return;
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
                signal_worker_group(Some(pid), libc::SIGKILL);
                match status {
                    Ok(status) => eprintln!("process={} pid={} exited status={status}", spec.id, pid),
                    Err(error) => eprintln!("process={} pid={} wait failed: {error}", spec.id, pid),
                }
            },
            _ = shutdown.changed() => {
                stop_child(&mut child, &spec.id).await;
                return;
            }
        }
        if started.elapsed() >= reset_after {
            backoff = initial_backoff;
        }
        eprintln!("process={} retrying in {:?}", spec.id, backoff);
        wait_or_shutdown(backoff, &mut shutdown).await;
        if stopping(&shutdown) {
            return;
        }
        backoff = (backoff.saturating_mul(2)).min(max_backoff);
    }
}

fn stopping(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow() || shutdown.has_changed().is_err()
}

async fn wait_or_shutdown(delay: Duration, shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = sleep(delay) => {},
        changed = shutdown.changed() => if changed.is_ok() && *shutdown.borrow() {},
    }
}

#[cfg(unix)]
pub(crate) async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("cannot install SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("cannot install SIGINT handler");
    tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
}

#[cfg(not(unix))]
pub(crate) async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

pub(crate) struct RunningPlan {
    processes: HashMap<String, RunningProcess>,
    files: HashMap<PathBuf, String>,
}

struct RunningProcess {
    spec: ProcessSpec,
    shutdown_tx: watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

impl RunningProcess {
    fn spawn(spec: ProcessSpec) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(supervise(spec.clone(), shutdown_rx));
        Self {
            spec,
            shutdown_tx,
            handle,
        }
    }
}

fn files_by_path(files: Vec<GeneratedFile>) -> HashMap<PathBuf, String> {
    files
        .into_iter()
        .map(|item| (item.path, item.content))
        .collect()
}

fn references_path(spec: &ProcessSpec, path: &Path) -> bool {
    spec.argv.iter().skip(1).any(|arg| {
        Path::new(arg) == path
            || arg
                .split_once('=')
                .is_some_and(|(_, value)| Path::new(value) == path)
    })
}

pub(crate) async fn reconcile_plan(
    running: &mut RunningPlan,
    plan: CompiledPlan,
    restart_management: bool,
) {
    let next_files = files_by_path(plan.generated_files);
    let changed_paths: Vec<_> = running
        .files
        .keys()
        .chain(next_files.keys())
        .filter(|path| running.files.get(*path) != next_files.get(*path))
        .collect();
    let mut previous = std::mem::take(&mut running.processes);
    let mut replacements = Vec::new();
    for spec in plan.processes {
        let management_changed = restart_management
            && (spec.id.starts_with("agent-lane-") || spec.id.starts_with("agent-seed-"));
        if !management_changed
            && !changed_paths
                .iter()
                .any(|path| references_path(&spec, path))
            && previous
                .get(&spec.id)
                .is_some_and(|process| process.spec == spec && !process.handle.is_finished())
            && let Some(process) = previous.remove(&spec.id)
        {
            running.processes.insert(spec.id, process);
        } else {
            replacements.push(spec);
        }
    }
    stop_processes(previous).await;
    for path in running
        .files
        .keys()
        .filter(|path| !next_files.contains_key(*path))
    {
        if let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "cannot remove obsolete generated file {}: {error}",
                path.display()
            );
        }
    }
    for spec in replacements {
        running
            .processes
            .insert(spec.id.clone(), RunningProcess::spawn(spec));
    }
    running.files = next_files;
}

pub(crate) async fn start_plan(plan: CompiledPlan) -> Result<RunningPlan, String> {
    manifest::write_generated_files(&plan)?;
    Ok(RunningPlan {
        processes: plan
            .processes
            .into_iter()
            .map(|spec| (spec.id.clone(), RunningProcess::spawn(spec)))
            .collect(),
        files: files_by_path(plan.generated_files),
    })
}

pub(crate) async fn stop_plan(plan: &mut RunningPlan) {
    stop_processes(std::mem::take(&mut plan.processes)).await;
}

async fn stop_processes(processes: HashMap<String, RunningProcess>) {
    for process in processes.values() {
        let _ = process.shutdown_tx.send(true);
    }
    for process in processes.into_values() {
        let _ = process.handle.await;
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

pub(crate) async fn run(plan: CompiledPlan) -> Result<(), String> {
    let mut running = start_plan(plan).await?;
    wait_for_shutdown().await;
    eprintln!("shutdown signal received");
    stop_plan(&mut running).await;
    Ok(())
}
