use std::{path::Path, process::Stdio, time::Duration};

use tokio::{
    process::Command,
    signal::unix::{SignalKind, signal},
    time::timeout,
};

pub(crate) async fn run(binary: &Path, config: &Path) -> Result<(), String> {
    // OpenSSH session processes change process group. Adopt them if the listener exits.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "cannot supervise HPNSSH sessions: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut terminate = signal(SignalKind::terminate()).map_err(|error| error.to_string())?;
    let mut child = Command::new(binary)
        .args(["-D", "-e", "-f"])
        .arg(config)
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot launch HPNSSH listener: {error}"))?;
    let result = tokio::select! {
        status = child.wait() => status
            .map_err(|error| format!("cannot wait for HPNSSH listener: {error}"))
            .and_then(|status| if status.success() { Ok(()) } else { Err(format!("HPNSSH listener exited: {status}")) }),
        _ = terminate.recv() => {
            if let Some(pid) = child.id() {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            }
            if timeout(Duration::from_secs(1), child.wait()).await.is_err() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            Ok(())
        }
    };
    reap_sessions()?;
    result
}

fn reap_sessions() -> Result<(), String> {
    let pid = std::process::id();
    let path = format!("/proc/self/task/{pid}/children");
    loop {
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot enumerate HPNSSH sessions: {error}"))?;
        let children = contents
            .split_whitespace()
            .filter_map(|word| word.parse::<libc::pid_t>().ok().filter(|pid| *pid > 0))
            .collect::<Vec<_>>();
        if children.is_empty() {
            return Ok(());
        }
        for child in &children {
            unsafe { libc::kill(*child, libc::SIGKILL) };
        }
        for child in children {
            loop {
                if unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) } >= 0
                    || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    break;
                }
            }
        }
    }
}
