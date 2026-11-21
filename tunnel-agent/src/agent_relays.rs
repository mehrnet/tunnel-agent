use std::{collections::HashSet, path::Path, thread};

use crate::{
    agent::read_secret_file,
    agent_lane::{AgentLaneRole, authority},
    agent_poll::{CONFIG_REQUEST_TIMEOUT, RemoteDocument},
    agent_spec::{self, AgentSpec},
    relay, seed,
    seed_client::SeedSource,
};

const MAX_PARALLEL_REQUESTS: usize = 8;

pub(crate) fn sources(spec: &AgentSpec) -> Vec<String> {
    let mut seen = HashSet::new();
    agent_spec::selected_lanes(spec)
        .into_iter()
        .filter(|lane| lane.selected && lane.role == AgentLaneRole::Server)
        .map(|lane| {
            format!(
                "http://{}",
                authority(&lane.listener.bind, lane.listener.port)
            )
        })
        .chain(spec.bundle_seed.iter().map(|seed| seed.source.clone()))
        .filter(|source| seen.insert(source.clone()))
        .collect()
}

pub(crate) fn first_working<T>(
    sources: &[String],
    attempt: impl Fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    let mut error = "no management relays configured".to_owned();
    for source in sources {
        match attempt(source) {
            Ok(result) => return Ok(result),
            Err(failure) => error = format!("{source}: {failure}"),
        }
    }
    Err(error)
}

pub(crate) fn collect<T: Send>(
    sources: &[String],
    attempt: impl Fn(&str) -> Result<T, String> + Sync,
) -> Vec<Result<T, String>> {
    sources
        .chunks(MAX_PARALLEL_REQUESTS)
        .flat_map(|chunk| {
            thread::scope(|scope| {
                let attempt = &attempt;
                let requests: Vec<_> = chunk
                    .iter()
                    .map(|source| {
                        scope.spawn(move || {
                            attempt(source).map_err(|error| format!("{source}: {error}"))
                        })
                    })
                    .collect();
                requests
                    .into_iter()
                    .map(|request| {
                        request
                            .join()
                            .unwrap_or_else(|_| Err("management relay request panicked".to_owned()))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

pub(crate) fn fetch_document(
    sources: &[String],
    node_id_file: &Path,
) -> Result<RemoteDocument, String> {
    let node_id = read_secret_file(node_id_file, "node id")?;
    let results = collect(sources, |source| {
        let raw =
            SeedSource::with_timeout(source, CONFIG_REQUEST_TIMEOUT)?.relay_document(&node_id)?;
        let verified = relay::verify_for_local_node(
            &raw,
            Path::new("/etc/tunnel-agent/relay-state.json"),
            node_id_file,
        )?;
        Ok(RemoteDocument {
            revision: verified.revision,
            manifest_json: verified.manifest_json,
        })
    });
    newest(results)
}

fn newest(results: Vec<Result<RemoteDocument, String>>) -> Result<RemoteDocument, String> {
    let mut latest: Option<RemoteDocument> = None;
    let mut error = "no management relays configured".to_owned();
    for result in results {
        match result {
            Ok(document)
                if latest
                    .as_ref()
                    .is_none_or(|active| document.revision > active.revision) =>
            {
                latest = Some(document);
            }
            Ok(_) => {}
            Err(failure) => error = failure,
        }
    }
    latest.ok_or(error)
}

pub(crate) fn heartbeat(sources: &[String], node_id: &str) -> Result<(), String> {
    first_working(sources, |source| seed::ping_relay(source, node_id))
}

#[cfg(test)]
#[path = "agent_relays_tests.rs"]
pub(crate) mod tests;
