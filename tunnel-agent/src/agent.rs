use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::blocking::Client;

use crate::{
    agent_control,
    agent_poll::{self, OnlineApi, PollJob, RemoteDocument, polling_interval, set_period},
    foreign_update,
    identity::valid_uuid,
    manifest::{self, Config},
    relay,
    runtime::{self, reconcile_plan, start_plan, stop_plan},
    seed,
    seed_client::SeedSource,
    telemetry,
};

pub(crate) struct AgentSettings {
    pub(crate) config_path: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) api_file: PathBuf,
    pub(crate) node_id_file: PathBuf,
    pub(crate) node_token_file: PathBuf,
    pub(crate) poll_interval_secs: Option<u64>,
    pub(crate) offline: bool,
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

fn online_api(settings: &AgentSettings) -> Result<OnlineApi, String> {
    let url = read_secret_file(&settings.api_file, "API URL")?;
    if !url.starts_with("https://") {
        return Err("API URL must use HTTPS".to_owned());
    }
    let node_id = read_secret_file(&settings.node_id_file, "node id")?;
    let token = read_secret_file(&settings.node_token_file, "node token")?;
    validate_node_credentials(&node_id, &token)?;
    fs::create_dir_all(&settings.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", settings.state_dir.display()))?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("cannot build API client: {error}"))?;
    Ok(OnlineApi {
        client,
        url,
        node_id,
        token,
        state_dir: settings.state_dir.clone(),
    })
}

async fn reconcile_online(settings: &AgentSettings, mut config: Config) -> Result<(), String> {
    let report_period = effective_poll_interval(settings, &config)?;
    let api = online_api(settings)?;
    api.prepare(config.clone()).await?;
    let (mut running, reporter) = start_agent_plan(settings, &config).await?;
    let mut revision = read_revision(&settings.state_dir.join("document-revision"))?;
    let mut etag = (revision != 0).then(|| format!("\"{revision}\""));
    let mut relay_etag = None;
    let mut config_poll = polling_interval(config.agent.config_poll_interval_secs);
    let mut reports = polling_interval(report_period);
    let mut updates = polling_interval(300);
    let mut desired = PollJob::new();
    let mut relayed = PollJob::new();
    let mut reporting = PollJob::new();
    let mut updater = foreign_update::Poller::new();
    let shutdown = runtime::wait_for_shutdown();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = updates.tick() => updater.start(settings, &config, api.credentials()),
            result = updater.result() => match result {
                Ok(Some(version)) => {
                    eprintln!("activated foreign bundle version={version}; restarting agent");
                    break;
                }
                Ok(None) => {},
                Err(error) => eprintln!("foreign update failed; retaining active bundle: {error}"),
            },
            _ = config_poll.tick() => {
                let (client, tag) = (api.clone(), etag.clone());
                desired.start(move || client.fetch_document(tag));
                if agent_control::has_active_client(&config) {
                    let (client, tag) = (api.clone(), relay_etag.clone());
                    relayed.start(move || client.refresh_relay(tag));
                } else {
                    relay_etag = None;
                }
            }
            result = desired.result() => match result {
                Ok(Some((document, tag))) if document.revision > revision => {
                    match apply_online_document(settings, &mut running, &config, revision, &document, &api).await {
                        Ok(next) => {
                            if config.agent.lanes_changed(&next.agent) {
                                reports.reset_immediately();
                                config_poll.reset_immediately();
                            }
                            config = next;
                            revision = document.revision;
                            etag = Some(tag);
                            set_period(&mut config_poll, config.agent.config_poll_interval_secs);
                            set_period(&mut reports, effective_poll_interval(settings, &config)?);
                        }
                        Err(error) => eprintln!("document revision={} rejected; retaining revision={revision}: {error}", document.revision),
                    }
                }
                Ok(Some((_, tag))) => etag = Some(tag),
                Ok(None) => {},
                Err(error) => eprintln!("document refresh failed; retaining revision={revision}: {error}"),
            },
            result = relayed.result() => match result {
                Ok(tag) => relay_etag = tag,
                Err(error) => eprintln!("relay cache refresh failed: {error}"),
            },
            _ = reports.tick() => {
                let (client, config, reporter) = (api.clone(), config.clone(), reporter.clone());
                reporting.start(move || client.report(&config, &reporter));
            }
            result = reporting.result() => {
                if let Err(error) = result {
                    eprintln!("telemetry report failed: {error}");
                }
            }
        }
    }
    stop_plan(&mut running).await;
    Ok(())
}

async fn apply_online_document(
    settings: &AgentSettings,
    running: &mut runtime::RunningPlan,
    active: &Config,
    revision: u64,
    document: &RemoteDocument,
    api: &OnlineApi,
) -> Result<Config, String> {
    let candidate = manifest::parse_config(&document.manifest_json)?;
    ensure_compatible_document(active, &candidate)?;
    if active.agent.lanes_changed(&candidate.agent) {
        api.prepare(candidate.clone()).await?;
    }
    apply_document(settings, running, active, revision, document, candidate).await
}

async fn reconcile_offline(settings: &AgentSettings, mut config: Config) -> Result<(), String> {
    let report_period = effective_poll_interval(settings, &config)?;
    let mut relay = relay_client(&config)?;
    let (mut running, reporter) = start_agent_plan(settings, &config).await?;
    let mut revision = read_revision(&settings.state_dir.join("document-revision"))?;
    let mut config_poll = polling_interval(config.agent.config_poll_interval_secs);
    let mut updates = polling_interval(offline_poll_interval(&config));
    let mut reports = polling_interval(report_period);
    let mut desired = PollJob::new();
    let mut updater = PollJob::new();
    let mut reporting = PollJob::new();
    let shutdown = runtime::wait_for_shutdown();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = config_poll.tick() => {
                if let Some(client) = &relay {
                    let (client, node_file) = (client.clone(), settings.node_id_file.clone());
                    desired.start(move || fetch_relay_document(&client, &node_file));
                }
            }
            result = desired.result() => match result {
                Ok(document) if document.revision > revision => {
                    match apply_offline_document(settings, &mut running, &config, revision, &document, &relay).await {
                        Ok((next, next_relay)) => {
                            let source_changed = seed_source(&config) != seed_source(&next);
                            relay = next_relay;
                            config = next;
                            revision = document.revision;
                            set_period(&mut config_poll, config.agent.config_poll_interval_secs);
                            set_period(&mut updates, offline_poll_interval(&config));
                            if source_changed {
                                updates.reset_immediately();
                            }
                            set_period(&mut reports, effective_poll_interval(settings, &config)?);
                        }
                        Err(error) => eprintln!("relay document revision={} rejected: {error}", document.revision),
                    }
                }
                Ok(_) => {},
                Err(error) => eprintln!("relay refresh failed; retaining revision={revision}: {error}"),
            },
            _ = updates.tick() => {
                let config = config.clone();
                updater.start(move || seed::stage_configured(&config));
            }
            result = updater.result() => match seed::activate_configured(&config, result) {
                Ok(Some(version)) => {
                    eprintln!("activated seeded bundle version={version}; restarting agent");
                    break;
                }
                Ok(None) => {},
                Err(error) => eprintln!("seed update failed; retaining active bundle: {error}"),
            },
            _ = reports.tick() => {
                let (node_file, config, reporter) = (settings.node_id_file.clone(), config.clone(), reporter.clone());
                reporting.start(move || report_domestic_telemetry(&node_file, &config, &reporter));
            }
            result = reporting.result() => {
                if let Err(error) = result {
                    eprintln!("domestic telemetry report failed: {error}");
                }
            }
        }
    }
    stop_plan(&mut running).await;
    Ok(())
}

async fn start_agent_plan(
    settings: &AgentSettings,
    config: &Config,
) -> Result<(runtime::RunningPlan, agent_poll::Reporter), String> {
    let bundle = config
        .bundle
        .as_ref()
        .ok_or_else(|| "agent manifest has no bundle".to_owned())?;
    let keys = crate::transport_identity::provision(&bundle.root, &settings.state_dir);
    let mut boot = config.clone();
    boot.tunnels.retain(|tunnel| {
        if let manifest::TunnelSpec::ManagedReverse(lane) = tunnel
            && lane.enabled
            && !lane.matches_identity(&keys)
        {
            eprintln!(
                "skipping lane {} until its transport identity is reconciled",
                lane.id
            );
            return false;
        }
        true
    });
    let running = start_plan(manifest::compile(&boot, &settings.state_dir)?).await?;
    Ok((running, agent_poll::reporter(keys)))
}

fn report_domestic_telemetry(
    node_id_file: &Path,
    config: &Config,
    reporter: &agent_poll::Reporter,
) -> Result<(), String> {
    let Some(seed_config) = &config.agent.bundle_seed else {
        return Ok(());
    };
    let source = &seed_config.source;
    let node_id = read_secret_file(node_id_file, "node id")?;
    if let Err(error) = seed::ping_relay(source, &node_id) {
        eprintln!("relay heartbeat failed: {error}");
    }
    let Some(report) = agent_poll::sample(reporter, config)? else {
        return Ok(());
    };
    telemetry::send_domestic(source, &node_id, &report)
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

fn seed_source(config: &Config) -> Option<&str> {
    config
        .agent
        .bundle_seed
        .as_ref()
        .map(|seed| seed.source.as_str())
}

fn relay_client(config: &Config) -> Result<Option<SeedSource>, String> {
    seed_source(config)
        .map(|source| SeedSource::with_timeout(source, agent_poll::CONFIG_REQUEST_TIMEOUT))
        .transpose()
}

async fn apply_offline_document(
    settings: &AgentSettings,
    running: &mut runtime::RunningPlan,
    active: &Config,
    revision: u64,
    document: &RemoteDocument,
    relay: &Option<SeedSource>,
) -> Result<(Config, Option<SeedSource>), String> {
    let candidate = manifest::parse_config(&document.manifest_json)?;
    let next_relay = if seed_source(active) == seed_source(&candidate) {
        relay.clone()
    } else {
        relay_client(&candidate)?
    };
    let config = apply_document(settings, running, active, revision, document, candidate).await?;
    Ok((config, next_relay))
}

fn fetch_relay_document(
    source: &SeedSource,
    node_id_file: &Path,
) -> Result<RemoteDocument, String> {
    let node_id = read_secret_file(node_id_file, "node id")?;
    let raw = source.relay_document(&node_id)?;
    let verified = relay::verify_for_local_node(
        &raw,
        Path::new("/etc/tunnel-agent/relay-state.json"),
        node_id_file,
    )?;
    Ok(RemoteDocument {
        revision: verified.revision,
        manifest_json: verified.manifest_json,
    })
}

async fn apply_document(
    settings: &AgentSettings,
    running: &mut crate::runtime::RunningPlan,
    active_config: &Config,
    active_revision: u64,
    document: &RemoteDocument,
    candidate_config: Config,
) -> Result<Config, String> {
    ensure_compatible_document(active_config, &candidate_config)?;
    let candidate_plan = manifest::compile(&candidate_config, &settings.state_dir)?;
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
        active_config.agent.lanes_changed(&candidate_config.agent),
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
