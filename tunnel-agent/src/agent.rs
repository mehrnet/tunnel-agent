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
    manifest::{self, Config},
    runtime::{self, spawn_plan, start_plan, stop_plan},
};

const MAX_DOCUMENT_BYTES: usize = 1_048_576;

pub(crate) struct AgentSettings {
    pub(crate) config_path: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) api_file: PathBuf,
    pub(crate) node_id_file: PathBuf,
    pub(crate) node_token_file: PathBuf,
    pub(crate) poll_interval_secs: u64,
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
    let valid_uuid = node_id.len() == 36
        && node_id.chars().enumerate().all(|(index, character)| {
            (matches!(index, 8 | 13 | 18 | 23) && character == '-') || character.is_ascii_hexdigit()
        });
    if !valid_uuid {
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
    if settings.poll_interval_secs < 5 {
        return Err("poll_interval_secs must be at least 5".to_owned());
    }
    let mut active_config = manifest::load_config(&settings.config_path)?;
    if active_config.version != 3 {
        return Err("agent requires a version 3 bootstrap manifest".to_owned());
    }
    if settings.offline {
        return runtime::run(manifest::compile(&active_config, &settings.state_dir)?).await;
    }
    let api_url = read_secret_file(&settings.api_file, "API URL")?;
    if !api_url.starts_with("https://") {
        return Err("API URL must use HTTPS".to_owned());
    }
    let node_id = read_secret_file(&settings.node_id_file, "node id")?;
    let node_token = read_secret_file(&settings.node_token_file, "node token")?;
    validate_node_credentials(&node_id, &node_token)?;

    let initial_plan = manifest::compile(&active_config, &settings.state_dir)?;
    let mut running = start_plan(initial_plan).await?;
    fs::create_dir_all(&settings.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", settings.state_dir.display()))?;
    let revision_path = settings.state_dir.join("document-revision");
    let mut active_revision = read_revision(&revision_path)?;
    let mut etag = (active_revision != 0).then(|| format!("\"{active_revision}\""));
    let client = BlockingClient::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("cannot build API client: {error}"))?;
    let mut poll = interval(Duration::from_secs(settings.poll_interval_secs));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = crate::runtime::wait_for_shutdown();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
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
                        match apply_document(&settings, &mut running, &active_config, active_revision, &document).await {
                            Ok(config) => {
                                active_config = config;
                                active_revision = document.revision;
                                etag = Some(new_etag);
                            }
                            Err(error) => eprintln!(
                                "document revision={} rejected; retaining revision={active_revision}: {error}",
                                document.revision
                            ),
                        }
                    }
                    Err(error) => eprintln!("document refresh failed; retaining revision={active_revision}: {error}"),
                }
            }
        }
    }
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
    stop_plan(running).await;
    *running = spawn_plan(candidate_plan);
    manifest::write_config_atomic(
        &settings.state_dir.join("document-revision"),
        &document.revision.to_string(),
    )?;
    Ok(candidate_config)
}

#[cfg(test)]
mod tests {
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
