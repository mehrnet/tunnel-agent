use std::{process::Stdio, time::Duration};

use tokio::{
    process::{Child, Command},
    sync::watch,
    time::{Instant, sleep, timeout},
};

use crate::manifest::{self, CompiledPlan, ProcessSpec};

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
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
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
                wait_or_shutdown(backoff, &mut shutdown).await;
                if *shutdown.borrow() {
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
        wait_or_shutdown(backoff, &mut shutdown).await;
        if *shutdown.borrow() {
            return;
        }
        backoff = (backoff.saturating_mul(2)).min(max_backoff);
    }
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
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

pub(crate) fn spawn_plan(plan: CompiledPlan) -> RunningPlan {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handles = plan
        .processes
        .into_iter()
        .map(|process| tokio::spawn(supervise(process, shutdown_rx.clone())))
        .collect();
    RunningPlan {
        shutdown_tx,
        handles,
    }
}

pub(crate) async fn start_plan(plan: CompiledPlan) -> Result<RunningPlan, String> {
    manifest::write_generated_files(&plan)?;
    Ok(spawn_plan(plan))
}

pub(crate) async fn stop_plan(plan: &mut RunningPlan) {
    let _ = plan.shutdown_tx.send(true);
    for handle in plan.handles.drain(..) {
        let _ = handle.await;
    }
}

pub(crate) async fn run(plan: CompiledPlan) -> Result<(), String> {
    let mut running = start_plan(plan).await?;
    wait_for_shutdown().await;
    eprintln!("shutdown signal received");
    stop_plan(&mut running).await;
    Ok(())
}
