use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::release::{
    BundleFile, BundleIndex, INDEX_NAME, ReleaseDocument, as_utf8, cache_link_or_copy,
    ensure_index_matches_document, local_architecture, parse_index, set_mode, sha256_bytes,
    unique_suffix, valid_hash, verify_file, verify_release_document,
};
use crate::seed_client::SeedSource;

const MAX_DOCUMENT_BYTES: u64 = 16_384;
const MAX_INDEX_BYTES: u64 = 1_048_576;

pub(crate) struct SeedResult {
    pub(crate) version: String,
    pub(crate) downloaded_files: usize,
    pub(crate) reused_files: usize,
    pub(crate) changed: bool,
    pub(crate) activated: bool,
}

struct ServeState {
    document: Vec<u8>,
    index: Vec<u8>,
    blobs: HashMap<String, PathBuf>,
}

pub(crate) fn serve(
    root: &Path,
    listener: SocketAddr,
    document_path: &Path,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    if !listener.ip().is_loopback() {
        return Err("seed listener must bind to a loopback address".to_owned());
    }
    let mut state = load_serve_state(root, document_path)?;
    let socket = TcpListener::bind(listener)
        .map_err(|error| format!("cannot bind seed listener {listener}: {error}"))?;
    eprintln!("serving bundle seed on {listener}");
    for connection in socket.incoming() {
        match connection {
            Ok(stream) => {
                if fs::read(document_path).ok().as_deref() != Some(&state.document) {
                    state = load_serve_state(root, document_path)?;
                    eprintln!("reloaded bundle seed release document");
                }
                if let Err(error) = serve_connection(stream, &state, relay_directory) {
                    eprintln!("seed request failed: {error}");
                }
            }
            Err(error) => eprintln!("seed listener accept failed: {error}"),
        }
    }
    Ok(())
}

pub(crate) fn fetch_relay_document(source: &str, node_id: &str) -> Result<String, String> {
    if !valid_node_id(node_id) {
        return Err("relay node id is invalid".to_owned());
    }
    let source = SeedSource::new(source)?;
    let raw = source.get_bytes(&format!("/v1/relay/{node_id}"), MAX_DOCUMENT_BYTES)?;
    String::from_utf8(raw).map_err(|_| "relay response is not UTF-8".to_owned())
}

pub(crate) fn ping_relay(source: &str, node_id: &str) -> Result<(), String> {
    if !valid_node_id(node_id) {
        return Err("relay node id is invalid".to_owned());
    }
    SeedSource::new(source)?.get_bytes(&format!("/v1/ping/{node_id}"), 0)?;
    Ok(())
}

pub(crate) fn apply(root: &Path, source: &str, activate: bool) -> Result<SeedResult, String> {
    let source = SeedSource::new(source)?;
    let architecture = local_architecture()?;
    let raw_document = source.get_bytes("/v1/release", MAX_DOCUMENT_BYTES)?;
    let document =
        verify_release_document(as_utf8(&raw_document, "release document")?, &architecture)?;
    let raw_index = source.get_bytes("/v1/index", MAX_INDEX_BYTES)?;
    if sha256_bytes(&raw_index) != document.inventory_sha256 {
        return Err("seed inventory does not match the signed release document".to_owned());
    }
    let index = parse_index(as_utf8(&raw_index, "bundle inventory")?)?;
    ensure_index_matches_document(&index, &document)?;
    install_seeded_release(root, &source, &document, &index, &raw_index, activate)
}

pub(crate) fn restart_service(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@._-".contains(&byte))
    {
        return Err("service name contains unsafe characters".to_owned());
    }
    let status = Command::new("systemctl")
        .arg("restart")
        .arg(name)
        .status()
        .map_err(|error| format!("cannot invoke systemctl: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl restart {name} exited with {status}"))
    }
}

fn load_serve_state(root: &Path, document_path: &Path) -> Result<ServeState, String> {
    let architecture = local_architecture()?;
    let document = fs::read(document_path)
        .map_err(|error| format!("cannot read {}: {error}", document_path.display()))?;
    let release = verify_release_document(
        as_utf8(&document, "cached release document")?,
        &architecture,
    )?;
    let current = root.join("current");
    let index = fs::read(current.join(INDEX_NAME))
        .map_err(|error| format!("cannot read current bundle index: {error}"))?;
    if sha256_bytes(&index) != release.inventory_sha256 {
        return Err("current bundle index does not match the cached release document".to_owned());
    }
    let parsed = parse_index(as_utf8(&index, "current bundle inventory")?)?;
    ensure_index_matches_document(&parsed, &release)?;
    let blobs = parsed
        .files
        .into_iter()
        .try_fold(HashMap::new(), |mut blobs, file| {
            let path = current.join(&file.path);
            verify_file(&path, &file)?;
            blobs.entry(file.sha256).or_insert(path);
            Ok::<_, String>(blobs)
        })?;
    Ok(ServeState {
        document,
        index,
        blobs,
    })
}

fn serve_connection(
    stream: TcpStream,
    state: &ServeState,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader
        .read_line(&mut request)
        .map_err(|error| format!("cannot read seed request: {error}"))?;
    drain_headers(&mut reader)?;
    let mut stream = reader.into_inner();
    if request.split_whitespace().next() != Some("GET") {
        return write_response(&mut stream, 405, "text/plain", b"method not allowed\n");
    }
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    match path {
        "/v1/release" => write_response(&mut stream, 200, "application/json", &state.document),
        "/v1/index" => write_response(&mut stream, 200, "application/json", &state.index),
        value if value.starts_with("/v1/relay/") => {
            serve_relay(&mut stream, value, relay_directory)
        }
        value if value.starts_with("/v1/ping/") => serve_ping(&mut stream, value, relay_directory),
        _ => serve_blob(&mut stream, path, state),
    }
}

fn serve_relay(
    stream: &mut TcpStream,
    path: &str,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let Some(node_id) = path
        .strip_prefix("/v1/relay/")
        .filter(|value| valid_node_id(value))
    else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    let Some(directory) = relay_directory else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    match fs::read(directory.join(format!("{node_id}.json"))) {
        Ok(document) if document.len() as u64 <= MAX_DOCUMENT_BYTES => {
            record_relay_heartbeat(directory, node_id)?;
            write_response(stream, 200, "application/json", &document)
        }
        _ => write_response(stream, 404, "text/plain", b"not found\n"),
    }
}

fn serve_ping(
    stream: &mut TcpStream,
    path: &str,
    relay_directory: Option<&Path>,
) -> Result<(), String> {
    let Some(node_id) = path
        .strip_prefix("/v1/ping/")
        .filter(|value| valid_node_id(value))
    else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    let Some(directory) = relay_directory else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    if !directory.join(format!("{node_id}.json")).is_file() {
        return write_response(stream, 404, "text/plain", b"not found\n");
    }
    record_relay_heartbeat(directory, node_id)?;
    write_response(stream, 204, "text/plain", b"")
}

fn record_relay_heartbeat(relay_directory: &Path, node_id: &str) -> Result<(), String> {
    let state_directory = relay_directory
        .parent()
        .ok_or_else(|| "relay directory has no state parent".to_owned())?
        .join("relay-heartbeats");
    fs::create_dir_all(&state_directory)
        .map_err(|error| format!("cannot create relay heartbeat directory: {error}"))?;
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs();
    crate::release::write_atomic(
        &state_directory.join(format!("{node_id}.json")),
        format!(r#"{{"observed_at":{observed_at}}}"#).as_bytes(),
    )
}

fn serve_blob(stream: &mut TcpStream, path: &str, state: &ServeState) -> Result<(), String> {
    let hash = path
        .strip_prefix("/v1/blob/")
        .filter(|value| valid_hash(value));
    let Some(hash) = hash else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    let Some(source) = state.blobs.get(hash) else {
        return write_response(stream, 404, "text/plain", b"not found\n");
    };
    let size = fs::metadata(source)
        .map_err(|error| format!("cannot stat seed blob: {error}"))?
        .len();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n")
        .map_err(|error| format!("cannot write seed response: {error}"))?;
    let mut source =
        File::open(source).map_err(|error| format!("cannot open seed blob: {error}"))?;
    std::io::copy(&mut source, stream)
        .map_err(|error| format!("cannot write seed blob: {error}"))?;
    Ok(())
}

fn drain_headers(reader: &mut BufReader<TcpStream>) -> Result<(), String> {
    let mut bytes = 0_usize;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| format!("cannot read seed request headers: {error}"))?;
        bytes += line.len();
        if bytes > 8_192 {
            return Err("seed request headers exceed 8192 bytes".to_owned());
        }
        if line == "\r\n" || line.is_empty() {
            return Ok(());
        }
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    write!(stream, "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n", body.len())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| format!("cannot write seed response: {error}"))
}

fn install_seeded_release(
    root: &Path,
    source: &SeedSource,
    document: &ReleaseDocument,
    index: &BundleIndex,
    raw_index: &[u8],
    activate: bool,
) -> Result<SeedResult, String> {
    let release_dir = root.join("releases").join(&document.version);
    if release_dir.exists() {
        verify_installed_release(&release_dir, index, raw_index, document)?;
        let changed = activate && !is_current_release(root, &release_dir);
        if activate {
            activate_release(root, &release_dir)?;
        }
        return Ok(SeedResult {
            version: document.version.clone(),
            downloaded_files: 0,
            reused_files: index.files.len(),
            changed,
            activated: activate,
        });
    }
    let cache = root.join("cache").join("blobs");
    fs::create_dir_all(&cache).map_err(|error| format!("cannot create blob cache: {error}"))?;
    prime_cache_from_current(root, &cache)?;
    let stage = root
        .join("releases")
        .join(format!(".{}-{}", document.version, unique_suffix()?));
    fs::create_dir_all(&stage).map_err(|error| format!("cannot create release stage: {error}"))?;
    let result = materialize_stage(&stage, &cache, source, index, raw_index);
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    let (downloaded_files, reused_files) = result?;
    fs::rename(&stage, &release_dir)
        .map_err(|error| format!("cannot publish seeded release: {error}"))?;
    if activate {
        activate_release(root, &release_dir)?;
    }
    Ok(SeedResult {
        version: document.version.clone(),
        downloaded_files,
        reused_files,
        changed: true,
        activated: activate,
    })
}

fn materialize_stage(
    stage: &Path,
    cache: &Path,
    source: &SeedSource,
    index: &BundleIndex,
    raw_index: &[u8],
) -> Result<(usize, usize), String> {
    let mut downloaded = 0_usize;
    let mut reused = 0_usize;
    for entry in &index.files {
        let blob = cache.join(&entry.sha256);
        if verify_file(&blob, entry).is_err() {
            download_blob(source, entry, &blob)?;
            downloaded += 1;
        } else {
            reused += 1;
        }
        let destination = stage.join(&entry.path);
        let parent = destination
            .parent()
            .ok_or_else(|| "bundle path has no parent".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create bundle directory: {error}"))?;
        copy_blob(&blob, &destination)?;
        set_mode(&destination, entry.mode)?;
        verify_file(&destination, entry)?;
    }
    fs::write(stage.join(INDEX_NAME), raw_index)
        .map_err(|error| format!("cannot write bundle index: {error}"))?;
    Ok((downloaded, reused))
}

fn prime_cache_from_current(root: &Path, cache: &Path) -> Result<(), String> {
    let current = root.join("current");
    let Ok(raw_index) = fs::read(current.join(INDEX_NAME)) else {
        return Ok(());
    };
    let Ok(index_text) = std::str::from_utf8(&raw_index) else {
        return Ok(());
    };
    let Ok(index) = parse_index(index_text) else {
        return Ok(());
    };
    for entry in index.files {
        let source = current.join(&entry.path);
        let destination = cache.join(&entry.sha256);
        if !destination.exists() && verify_file(&source, &entry).is_ok() {
            cache_link_or_copy(&source, &destination)?;
        }
    }
    Ok(())
}

fn download_blob(
    source: &SeedSource,
    entry: &BundleFile,
    destination: &Path,
) -> Result<(), String> {
    let temporary = destination.with_extension(format!("part-{}", unique_suffix()?));
    source.get_file(
        &format!("/v1/blob/{}", entry.sha256),
        &temporary,
        entry.bytes,
    )?;
    if let Err(error) = verify_file(&temporary, entry) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, destination)
        .map_err(|error| format!("cannot cache seeded blob: {error}"))
}

fn copy_blob(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("cannot materialize bundle file: {error}"))
}

fn verify_installed_release(
    release_dir: &Path,
    index: &BundleIndex,
    raw_index: &[u8],
    document: &ReleaseDocument,
) -> Result<(), String> {
    let installed = fs::read(release_dir.join(INDEX_NAME))
        .map_err(|error| format!("existing release is missing its index: {error}"))?;
    if installed != raw_index || sha256_bytes(&installed) != document.inventory_sha256 {
        return Err("existing release does not match the signed inventory".to_owned());
    }
    for entry in &index.files {
        verify_file(&release_dir.join(&entry.path), entry)?;
    }
    Ok(())
}

fn activate_release(root: &Path, release: &Path) -> Result<(), String> {
    let next = root.join("current.next");
    let current = root.join("current");
    let _ = fs::remove_file(&next);
    #[cfg(unix)]
    std::os::unix::fs::symlink(release, &next)
        .map_err(|error| format!("cannot create next release symlink: {error}"))?;
    #[cfg(not(unix))]
    return Err("seed activation requires a Unix host".to_owned());
    fs::rename(next, current).map_err(|error| format!("cannot activate seeded release: {error}"))
}

fn is_current_release(root: &Path, release: &Path) -> bool {
    fs::canonicalize(root.join("current")).ok() == fs::canonicalize(release).ok()
}

fn valid_node_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            (matches!(index, 8 | 13 | 18 | 23) && byte == b'-') || byte.is_ascii_hexdigit()
        })
}
