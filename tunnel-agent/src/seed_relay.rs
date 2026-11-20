use std::{
    fs,
    net::TcpStream,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    identity::valid_uuid,
    release::write_atomic,
    seed::{MAX_RELAY_DOCUMENT_BYTES, write_response},
    telemetry,
};

pub(crate) fn serve_document(
    stream: &mut TcpStream,
    path: &str,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let Some((node_id, directory)) = relay_node(path, "/v1/relay/", relay_directory) else {
        return not_found(stream);
    };
    match fs::read(directory.join(format!("{node_id}.json"))) {
        Ok(document) if document.len() as u64 <= MAX_RELAY_DOCUMENT_BYTES => {
            write_response(stream, 200, "application/json", &document)
        }
        _ => not_found(stream),
    }
}

pub(crate) fn serve_ping(
    stream: &mut TcpStream,
    path: &str,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let Some((node_id, directory)) = relay_node(path, "/v1/ping/", relay_directory) else {
        return not_found(stream);
    };
    if !directory.join(format!("{node_id}.json")).is_file() {
        return not_found(stream);
    }
    record_heartbeat(directory, node_id)?;
    write_response(stream, 200, "text/plain", b"")
}

pub(crate) fn serve_telemetry(
    stream: &mut TcpStream,
    path: &str,
    body: &[u8],
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let Some((node_id, directory)) = relay_node(path, "/v1/telemetry/", relay_directory) else {
        return not_found(stream);
    };
    if !directory.join(format!("{node_id}.json")).is_file() {
        return not_found(stream);
    }
    if telemetry::parse_relay_report(body).is_err() {
        return write_response(stream, 400, "text/plain", b"invalid telemetry\n");
    }
    let directory = state_directory(directory, "relay-telemetry")?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create relay telemetry directory: {error}"))?;
    write_atomic(&directory.join(format!("{node_id}.json")), body)?;
    write_response(stream, 202, "application/json", b"{}")
}

fn relay_node<'a>(
    path: &'a str,
    prefix: &str,
    relay_directory: Option<&'a Path>,
) -> Option<(&'a str, &'a Path)> {
    let node_id = path
        .strip_prefix(prefix)
        .filter(|value| valid_uuid(value))?;
    relay_directory.map(|directory| (node_id, directory))
}

fn record_heartbeat(relay_directory: &Path, node_id: &str) -> Result<(), String> {
    let directory = state_directory(relay_directory, "relay-heartbeats")?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create relay heartbeat directory: {error}"))?;
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs();
    write_atomic(
        &directory.join(format!("{node_id}.json")),
        format!(r#"{{"observed_at":{observed_at}}}"#).as_bytes(),
    )
}

fn state_directory(relay_directory: &Path, name: &str) -> Result<PathBuf, String> {
    relay_directory
        .parent()
        .ok_or_else(|| "relay directory has no state parent".to_owned())
        .map(|parent| parent.join(name))
}

fn not_found(stream: &mut TcpStream) -> Result<(), String> {
    write_response(stream, 404, "text/plain", b"not found\n")
}
