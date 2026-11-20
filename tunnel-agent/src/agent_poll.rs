use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{
    StatusCode,
    blocking::Client,
    header::{ETAG, IF_NONE_MATCH},
};
use serde::Deserialize;
use tokio::{
    task::JoinHandle,
    time::{Interval, MissedTickBehavior, interval},
};

use crate::{agent_control, manifest::Config, telemetry};

const MAX_DOCUMENT_BYTES: usize = 1_048_576;
pub(crate) const CONFIG_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
pub(crate) struct RemoteDocument {
    pub(crate) revision: u64,
    pub(crate) manifest_json: String,
}

pub(crate) struct PollJob<T>(Option<JoinHandle<Result<T, String>>>);

impl<T: Send + 'static> PollJob<T> {
    pub(crate) fn new() -> Self {
        Self(None)
    }

    pub(crate) fn start(&mut self, task: impl FnOnce() -> Result<T, String> + Send + 'static) {
        if self.0.is_none() {
            self.0 = Some(tokio::task::spawn_blocking(task));
        }
    }

    pub(crate) async fn result(&mut self) -> Result<T, String> {
        let Some(task) = &mut self.0 else {
            return std::future::pending().await;
        };
        let result = task.await;
        self.0 = None;
        result.map_err(|error| format!("poll task failed: {error}"))?
    }
}

pub(crate) fn polling_interval(seconds: u64) -> Interval {
    let mut poll = interval(Duration::from_secs(seconds));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    poll
}

pub(crate) fn set_period(poll: &mut Interval, seconds: u64) {
    if poll.period() != Duration::from_secs(seconds) {
        *poll = polling_interval(seconds);
        poll.reset();
    }
}

#[derive(Clone)]
pub(crate) struct OnlineApi {
    pub(crate) client: Client,
    pub(crate) url: String,
    pub(crate) node_id: String,
    pub(crate) token: String,
    pub(crate) state_dir: PathBuf,
}

impl OnlineApi {
    pub(crate) fn credentials(&self) -> (&str, &str, &str) {
        (&self.url, &self.node_id, &self.token)
    }

    pub(crate) async fn prepare(&self, config: Config) -> Result<(), String> {
        let api = self.clone();
        tokio::task::spawn_blocking(move || {
            agent_control::prepare(&config, &api.client, &api.url, &api.node_id, &api.token)
        })
        .await
        .map_err(|error| format!("control preparation task failed: {error}"))?
    }

    pub(crate) fn fetch_document(
        &self,
        etag: Option<String>,
    ) -> Result<Option<(RemoteDocument, String)>, String> {
        let endpoint = format!(
            "{}/v1/nodes/{}/document",
            self.url.trim_end_matches('/'),
            self.node_id
        );
        let mut request = self
            .client
            .get(endpoint)
            .bearer_auth(&self.token)
            .timeout(CONFIG_REQUEST_TIMEOUT);
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .map_err(|error| format!("cannot fetch desired document: {error}"))?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(None);
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
        let etag = response
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
        let etag = etag.unwrap_or_else(|| format!("\"{}\"", document.revision));
        Ok(Some((document, etag)))
    }

    pub(crate) fn refresh_relay(&self, etag: Option<String>) -> Result<Option<String>, String> {
        agent_control::refresh_relay_documents(
            &self.client,
            &self.url,
            &self.node_id,
            &self.token,
            &self.state_dir.join("relay-documents"),
            etag,
        )
    }

    pub(crate) fn report(&self, config: &Config, reporter: &Reporter) -> Result<(), String> {
        if let Err(error) = agent_control::refresh_housekeeping(
            config,
            &self.state_dir,
            &self.client,
            &self.url,
            &self.node_id,
            &self.token,
        ) {
            eprintln!("control housekeeping failed: {error}");
        }
        let Some(report) = sample(reporter, config)? else {
            return Ok(());
        };
        telemetry::send_direct(&self.client, &self.url, &self.node_id, &self.token, &report)
    }
}

pub(crate) type Reporter = Arc<Mutex<telemetry::Reporter>>;

pub(crate) fn reporter(keys: crate::transport_identity::TransportKeys, state: PathBuf) -> Reporter {
    Arc::new(Mutex::new(telemetry::Reporter::new(keys, state)))
}

pub(crate) fn sample(
    reporter: &Reporter,
    config: &Config,
) -> Result<Option<telemetry::TelemetryReport>, String> {
    reporter
        .lock()
        .map_err(|error| error.to_string())?
        .sample(config)
}
