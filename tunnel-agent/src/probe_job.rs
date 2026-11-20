use std::{
    fs,
    io::{self, Read},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::{
    blocking::Client,
    header::{HOST, RANGE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};

use crate::manifest::{Config, GeneratedFile, ProcessSpec, write_config_atomic};

pub(crate) const PERIOD: u64 = 30;
const SAMPLE_BYTES: u64 = 65_536;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Job {
    pub(crate) id: String,
    starts_at: u64,
    ends_at: u64,
    port: u16,
    hostname: String,
    path: String,
    expected_status: u16,
    download: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Progress {
    id: String,
    sequence: u32,
    passed: u32,
    failed: u32,
    streak: u32,
    longest_streak: u32,
    latency_total_ms: u64,
    latency_max_ms: u64,
    bytes: u64,
    download_ms: u64,
    last_error: Option<Failure>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    Timeout,
    Transport,
    Status,
    ShortBody,
}

enum Sample {
    Passed { elapsed_ms: u64, bytes: u64 },
    Failed(Failure),
}

impl Job {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let duration = self.ends_at.saturating_sub(self.starts_at);
        if !crate::identity::valid_uuid(&self.id)
            || self.starts_at == 0
            || !(600..=86_400).contains(&duration)
            || !duration.is_multiple_of(PERIOD)
            || self.port < 1024
            || !crate::ingress::valid_tls_name(&self.hostname)
            || !self.path.starts_with('/')
            || self.path.len() > 160
            || !self
                .path
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/._~-".contains(&c))
            || !(200..=599).contains(&self.expected_status)
            || (self.download && !matches!(self.expected_status, 200 | 206))
        {
            return Err("invalid HTTPS probe job".to_owned());
        }
        Ok(())
    }

    fn slot(&self, now: u64) -> Option<u32> {
        (self.starts_at..self.ends_at)
            .contains(&now)
            .then(|| ((now - self.starts_at) / PERIOD + 1) as u32)
    }
}

impl Progress {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            sequence: 0,
            passed: 0,
            failed: 0,
            streak: 0,
            longest_streak: 0,
            latency_total_ms: 0,
            latency_max_ms: 0,
            bytes: 0,
            download_ms: 0,
            last_error: None,
        }
    }

    fn record(&mut self, slot: u32, sample: Sample) {
        if self.sequence + 1 != slot {
            self.streak = 0;
        }
        self.sequence = slot;
        match sample {
            Sample::Passed { elapsed_ms, bytes } => {
                self.passed += 1;
                self.latency_total_ms += elapsed_ms;
                self.latency_max_ms = self.latency_max_ms.max(elapsed_ms);
                self.bytes += bytes;
                if bytes != 0 {
                    self.download_ms += elapsed_ms.max(1);
                }
                self.streak = 0;
                self.last_error = None;
            }
            Sample::Failed(reason) => {
                self.failed += 1;
                self.streak += 1;
                self.longest_streak = self.longest_streak.max(self.streak);
                self.last_error = Some(reason);
            }
        }
    }
}

fn progress_path(state: &Path) -> PathBuf {
    state.join("probe-progress.json")
}

pub(crate) fn report(state: &Path, job: Option<&Job>) -> Option<Progress> {
    let job = job?;
    let raw = fs::read(progress_path(state)).ok()?;
    let progress: Progress = serde_json::from_slice(&raw).ok()?;
    (progress.id == job.id && progress.sequence > 0).then_some(progress)
}

pub(crate) fn compile(
    config: &Config,
    state: &Path,
    workers: &mut Vec<ProcessSpec>,
    files: &mut Vec<GeneratedFile>,
) -> Result<(), String> {
    let Some(job) = &config.probe_job else {
        return Ok(());
    };
    let bundle = config
        .bundle
        .as_ref()
        .ok_or("probe jobs require a bundle")?;
    let file = state.join("generated/probe-job.json");
    workers.push(ProcessSpec {
        id: "agent-probe-job".to_owned(),
        argv: vec![
            bundle.root.join("bin/tunnel-agent").display().to_string(),
            "probe-job-worker".to_owned(),
            "--config".to_owned(),
            file.display().to_string(),
            "--state".to_owned(),
            state.display().to_string(),
        ],
        env: Default::default(),
        workdir: None,
        restart: Default::default(),
    });
    files.push(GeneratedFile {
        path: file,
        content: serde_json::to_string(job).map_err(|error| error.to_string())?,
    });
    Ok(())
}

pub(crate) fn run(file: &Path, state: &Path) -> Result<(), String> {
    let job: Job = serde_json::from_slice(&fs::read(file).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    job.validate()?;
    let mut progress = report(state, Some(&job)).unwrap_or_else(|| Progress::new(&job.id));
    let client = probe_client(&job)
        .build()
        .map_err(|error| error.to_string())?;
    let url = format!("https://{}:{}{}", job.hostname, job.port, job.path);
    loop {
        let now = unix_time()?;
        if let Some(slot) = job.slot(now).filter(|slot| *slot > progress.sequence) {
            progress.record(slot, measure(&client, &job, &url));
            let json = serde_json::to_string(&progress).map_err(|error| error.to_string())?;
            write_config_atomic(&progress_path(state), &json)?;
        }
        let delay = if now >= job.ends_at {
            300
        } else {
            let next = job
                .starts_at
                .saturating_add(u64::from(progress.sequence) * PERIOD);
            next.saturating_sub(unix_time()?).clamp(1, PERIOD)
        };
        std::thread::sleep(Duration::from_secs(delay));
    }
}

fn unix_time() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|error| error.to_string())
}

fn probe_client(job: &Job) -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .resolve(
            &job.hostname,
            SocketAddr::from((Ipv4Addr::LOCALHOST, job.port)),
        )
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
}

fn measure(client: &Client, job: &Job, url: &str) -> Sample {
    let start = Instant::now();
    let (response, bytes) = if job.download {
        (
            client
                .get(url)
                .header(HOST, &job.hostname)
                .header(RANGE, format!("bytes=0-{}", SAMPLE_BYTES - 1))
                .send(),
            SAMPLE_BYTES,
        )
    } else {
        (client.head(url).header(HOST, &job.hostname).send(), 0)
    };
    let response = match response {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Sample::Failed(Failure::Timeout),
        Err(_) => return Sample::Failed(Failure::Transport),
    };
    if response.status().as_u16() != job.expected_status {
        return Sample::Failed(Failure::Status);
    }
    if bytes > 0 {
        match io::copy(&mut response.take(bytes), &mut io::sink()) {
            Ok(count) if count == bytes => {}
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                return Sample::Failed(Failure::Timeout);
            }
            _ => return Sample::Failed(Failure::ShortBody),
        }
    }
    Sample::Passed {
        elapsed_ms: start.elapsed().as_millis().min(8_000) as u64,
        bytes,
    }
}

#[cfg(test)]
#[path = "probe_tls_tests.rs"]
mod tls_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_do_not_count_as_failures_or_join_streaks() {
        let mut progress = Progress::new("test");
        for slot in [1, 2] {
            progress.record(slot, Sample::Failed(Failure::Timeout));
        }
        let encoded = serde_json::to_string(&progress).unwrap();
        let mut progress: Progress = serde_json::from_str(&encoded).unwrap();
        for slot in [9, 10, 11] {
            progress.record(slot, Sample::Failed(Failure::Timeout));
        }
        progress.record(
            12,
            Sample::Passed {
                elapsed_ms: 15,
                bytes: SAMPLE_BYTES,
            },
        );
        assert_eq!(
            (progress.passed, progress.failed, progress.sequence),
            (1, 5, 12)
        );
        assert_eq!(
            (
                progress.streak,
                progress.longest_streak,
                progress.download_ms
            ),
            (0, 3, 15)
        );
        assert_eq!(progress.bytes, SAMPLE_BYTES);
    }
}
