use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::{
    StatusCode,
    blocking::Client as BlockingClient,
    header::{ETAG, IF_NONE_MATCH},
};
use serde::Deserialize;
use tokio::time::{MissedTickBehavior, interval};

use crate::{
    agent_control, agent_heartbeat, foreign_update,
    identity::valid_uuid,
    manifest::{self, Config},
    relay,
    runtime::{self, reconcile_plan, start_plan, stop_plan},
    seed, telemetry,
};

const MAX_DOCUMENT_BYTES: usize = 1_048_576;

pub(crate) struct AgentSettings {
    pub(crate) config_path: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) api_file: PathBuf,
    pub(crate) node_id_file: PathBuf,
    pub(crate) node_token_file: PathBuf,
    pub(crate) poll_interval_secs: Option<u64>,
    pub(crate) offline: bool,
}

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

pub(crate) fn read_secret_file(path: &Path, label: &str) -> Result<String, String> {
    let value = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(format!("{label} {} is empty", path.display()));
    }
    Ok(value)
}

fn validate_node_credentials(node_id: &str, token: &str) -> Result<(), String> {
    if !valid_uuid(node_id) {
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

pub(crate) async fn reconcile(settings: AgentSettings) -> Result<(), String> {
    let active_config = manifest::load_config(&settings.config_path)?;
    if active_config.version != 3 {
        return Err("agent requires a version 3 bootstrap manifest".to_owned());
    }
    if settings.offline {
        return reconcile_offline(&settings, active_config).await;
    }
    reconcile_online(&settings, active_config).await
}

async fn reconcile_online(
    settings: &AgentSettings,
    mut active_config: Config,
) -> Result<(), String> {
    let poll_interval_secs = effective_poll_interval(settings, &active_config)?;
    let api_url = read_secret_file(&settings.api_file, "API URL")?;
    if !api_url.starts_with("https://") {
        return Err("API URL must use HTTPS".to_owned());
    }
    let node_id = read_secret_file(&settings.node_id_file, "node id")?;
    let node_token = read_secret_file(&settings.node_token_file, "node token")?;
    validate_node_credentials(&node_id, &node_token)?;

    fs::create_dir_all(&settings.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", settings.state_dir.display()))?;
    let client = BlockingClient::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("cannot build API client: {error}"))?;
    agent_control::prepare(
        &active_config,
        &settings.state_dir,
        &client,
        &api_url,
        &node_id,
        &node_token,
    )?;
    let initial_plan = manifest::compile(&active_config, &settings.state_dir)?;
    let mut running = start_plan(initial_plan).await?;
    let revision_path = settings.state_dir.join("document-revision");
    let mut active_revision = read_revision(&revision_path)?;
    let mut etag = (active_revision != 0).then(|| format!("\"{active_revision}\""));
    let mut poll = polling_interval(poll_interval_secs);
    let mut updates = polling_interval(300);
    let mut updater = foreign_update::Poller::new();
    let mut telemetry_reporter = telemetry::Reporter::new();
    let shutdown = crate::runtime::wait_for_shutdown();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = updates.tick() => {
                updater.start(settings, &active_config, (&api_url, &node_id, &node_token));
            }
            result = updater.result() => {
                match result {
                    Ok(Some(version)) => {
                        eprintln!("activated foreign bundle version={version}; restarting agent");
                        stop_plan(&mut running).await;
                        return Ok(());
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("foreign update failed; retaining active bundle: {error}"),
                }
            }
            _ = &mut shutdown => {
                eprintln!("shutdown signal received");
                stop_plan(&mut running).await;
                return Ok(());
            }
            _ = poll.tick() => {
                let response = request_document(&client, &api_url, &node_id, &node_token, etag.as_deref()).await;
                match response {
                    Ok(DocumentFetch::Unchanged) => {},
                    Ok(DocumentFetch::Updated { document, etag: new_etag }) => {
                        if document.revision <= active_revision {
                            etag = Some(new_etag);
                            continue;
                        }
                        let preparation = manifest::parse_config(&document.manifest_json).and_then(|config| {
                            ensure_compatible_document(&active_config, &config)?;
                            agent_control::prepare(
                                &config,
                                &settings.state_dir,
                                &client,
                                &api_url,
                                &node_id,
                                &node_token,
                            )
                        });
                        if let Err(error) = preparation {
                            eprintln!("document revision={} rejected; retaining revision={active_revision}: {error}", document.revision);
                            continue;
                        }
                        match apply_document(settings, &mut running, &active_config, active_revision, &document).await {
                            Ok(config) => {
                                active_config = config;
                                active_revision = document.revision;
                                etag = Some(new_etag);
                                poll = polling_interval(effective_poll_interval(settings, &active_config)?);
                            }
                            Err(error) => eprintln!(
                                "document revision={} rejected; retaining revision={active_revision}: {error}",
                                document.revision
                            ),
                        }
                    }
                    Err(error) => eprintln!("document refresh failed; retaining revision={active_revision}: {error}"),
                }
                if let Err(error) = agent_control::refresh_cache(
                    &active_config,
                    &settings.state_dir,
                    &client,
                    &api_url,
                    &node_id,
                    &node_token,
                ) {
                    eprintln!("control cache refresh failed; retaining active cache: {error}");
                }
                if let Err(error) = report_online_telemetry(
                    &mut telemetry_reporter,
                    &active_config,
                    &client,
                    &api_url,
                    &node_id,
                    &node_token,
                ) {
                    eprintln!("telemetry report failed: {error}");
                }
            }
        }
    }
}

async fn reconcile_offline(settings: &AgentSettings, mut config: Config) -> Result<(), String> {
    let plan = manifest::compile(&config, &settings.state_dir)?;
    let mut running = start_plan(plan).await?;
    let revision_path = settings.state_dir.join("document-revision");
    let mut active_revision = read_revision(&revision_path)?;
    let mut updates = polling_interval(offline_poll_interval(&config));
    let mut heartbeat = polling_interval(effective_poll_interval(settings, &config)?);
    let mut telemetry_reporter = telemetry::Reporter::new();
    let shutdown = runtime::wait_for_shutdown();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                stop_plan(&mut running).await;
                return Ok(());
            }
            _ = updates.tick() => {
                match apply_relay_document(settings, &mut running, &config, active_revision).await {
                    Ok(Some((next_config, revision))) => {
                        active_revision = revision;
                        config = next_config;
                        updates = polling_interval(offline_poll_interval(&config));
                        heartbeat = polling_interval(effective_poll_interval(settings, &config)?);
                    }
                    Ok(None) => {},
                    Err(error) => eprintln!("relay refresh failed; retaining revision={active_revision}: {error}"),
                }
                match seed::apply_configured(&config).await {
                    Ok(Some(result)) if result.changed => {
                        eprintln!("activated seeded bundle version={}; restarting agent", result.version);
                        stop_plan(&mut running).await;
                        return Ok(());
                    }
                    Ok(_) => {},
                    Err(error) => eprintln!("seed update failed; retaining active bundle: {error}"),
                }
            }
            _ = heartbeat.tick() => {
                if let Err(error) = agent_heartbeat::send(settings, &config).await {
                    eprintln!("relay heartbeat failed: {error}");
                }
                if let Err(error) = report_domestic_telemetry(settings, &config, &mut telemetry_reporter).await {
                    eprintln!("domestic telemetry report failed: {error}");
                }
            }
        }
    }
}

fn report_online_telemetry(
    reporter: &mut telemetry::Reporter,
    config: &Config,
    client: &BlockingClient,
    api_url: &str,
    node_id: &str,
    node_token: &str,
) -> Result<(), String> {
    let Some(report) = reporter.sample(config)? else {
        return Ok(());
    };
    telemetry::send_direct(client, api_url, node_id, node_token, &report)
}

async fn report_domestic_telemetry(
    settings: &AgentSettings,
    config: &Config,
    reporter: &mut telemetry::Reporter,
) -> Result<(), String> {
    let Some(source) = config
        .agent
        .bundle_seed
        .as_ref()
        .map(|seed| seed.source.clone())
    else {
        return Ok(());
    };
    let Some(report) = reporter.sample(config)? else {
        return Ok(());
    };
    let node_id_file = settings.node_id_file.clone();
    tokio::task::spawn_blocking(move || {
        let node_id = read_secret_file(&node_id_file, "node id")?;
        telemetry::send_domestic(&source, &node_id, &report)
    })
    .await
    .map_err(|error| format!("domestic telemetry task failed: {error}"))?
}

fn offline_poll_interval(config: &Config) -> u64 {
    config
        .agent
        .bundle_seed
        .as_ref()
        .map(|seed| seed.poll_interval_secs)
        .unwrap_or(config.agent.poll_interval_secs)
}

fn effective_poll_interval(settings: &AgentSettings, config: &Config) -> Result<u64, String> {
    let interval = settings
        .poll_interval_secs
        .unwrap_or(config.agent.poll_interval_secs);
    if interval < 5 {
        return Err("poll_interval_secs must be at least 5".to_owned());
    }
    Ok(interval)
}

fn polling_interval(seconds: u64) -> tokio::time::Interval {
    let mut poll = interval(Duration::from_secs(seconds));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    poll
}

async fn request_document(
    client: &BlockingClient,
    api_url: &str,
    node_id: &str,
    node_token: &str,
    etag: Option<&str>,
) -> Result<DocumentFetch, String> {
    let client = client.clone();
    let api_url = api_url.to_owned();
    let node_id = node_id.to_owned();
    let node_token = node_token.to_owned();
    let etag = etag.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        fetch_document(&client, &api_url, &node_id, &node_token, etag.as_deref())
    })
    .await
    .map_err(|error| format!("document refresh task failed: {error}"))?
}

async fn apply_relay_document(
    settings: &AgentSettings,
    running: &mut crate::runtime::RunningPlan,
    active_config: &Config,
    active_revision: u64,
) -> Result<Option<(Config, u64)>, String> {
    let Some(source) = active_config
        .agent
        .bundle_seed
        .as_ref()
        .map(|seed| seed.source.clone())
    else {
        return Ok(None);
    };
    let node_id_file = settings.node_id_file.clone();
    let relay_state = PathBuf::from("/etc/tunnel-agent/relay-state.json");
    let verified = tokio::task::spawn_blocking(move || {
        let node_id = read_secret_file(&node_id_file, "node id")?;
        let raw = seed::fetch_relay_document(&source, &node_id)?;
        relay::verify_for_local_node(&raw, &relay_state, &node_id_file)
    })
    .await
    .map_err(|error| format!("relay refresh task failed: {error}"))??;
    if verified.revision <= active_revision {
        return Ok(None);
    }
    let document = RemoteDocument {
        revision: verified.revision,
        manifest_json: verified.manifest_json,
    };
    let config =
        apply_document(settings, running, active_config, active_revision, &document).await?;
    Ok(Some((config, document.revision)))
}

async fn apply_document(
    settings: &AgentSettings,
    running: &mut crate::runtime::RunningPlan,
    active_config: &Config,
    active_revision: u64,
    document: &RemoteDocument,
) -> Result<Config, String> {
    let (candidate_config, candidate_plan) = manifest::parse_config(&document.manifest_json)
        .and_then(|config| {
            ensure_compatible_document(active_config, &config)?;
            manifest::compile(&config, &settings.state_dir).map(|plan| (config, plan))
        })?;
    manifest::write_generated_files(&candidate_plan).and_then(|()| {
        manifest::write_config_atomic(&settings.config_path, &document.manifest_json)
    })?;
    eprintln!(
        "applying document revision={} replacing revision={active_revision}",
        document.revision
    );
    reconcile_plan(
        running,
        candidate_plan,
        active_config.agent != candidate_config.agent,
    )
    .await;
    manifest::write_config_atomic(
        &settings.state_dir.join("document-revision"),
        &document.revision.to_string(),
    )?;
    Ok(candidate_config)
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
