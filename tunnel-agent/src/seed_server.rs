use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use crate::{
    release::{
        INDEX_NAME, as_utf8, ensure_index_matches_document, local_architecture, parse_index,
        sha256_bytes, valid_hash, verify_file, verify_release_document,
    },
    seed::write_response,
    seed_relay,
};

const MAX_CONNECTIONS: usize = 16;
const MAX_BLOB_DOWNLOADS: usize = 8;
const MAX_HEADERS_BYTES: usize = 8_192;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TELEMETRY_BYTES: usize = 16_384;

struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn acquire(count: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        let mut active = count.load(Ordering::Relaxed);
        while active < limit {
            match count.compare_exchange_weak(
                active,
                active + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self(count.clone())),
                Err(observed) => active = observed,
            }
        }
        None
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

struct SeedServer {
    root: PathBuf,
    document_path: PathBuf,
    relay_directory: Option<PathBuf>,
    state: Mutex<Option<Arc<ServeState>>>,
    blobs: Arc<AtomicUsize>,
}

struct ServeState {
    document: Vec<u8>,
    index: Vec<u8>,
    blobs: HashMap<String, PathBuf>,
}

impl SeedServer {
    fn start_request(
        self: &Arc<Self>,
        stream: TcpStream,
        slots: &Arc<AtomicUsize>,
    ) -> std::io::Result<Option<thread::JoinHandle<()>>> {
        let Some(slot) = Slot::acquire(slots, MAX_CONNECTIONS) else {
            return Ok(None);
        };
        let server = self.clone();
        thread::Builder::new()
            .spawn(move || {
                let _slot = slot;
                if let Err(error) = serve_connection(stream, &server) {
                    eprintln!("seed request failed: {error}");
                }
            })
            .map(Some)
    }

    fn release_state(&self) -> Result<Arc<ServeState>, String> {
        let mut state = self.state.lock().map_err(|error| error.to_string())?;
        let doc = fs::read(&self.document_path).map_err(|error| error.to_string())?;
        if let Some(current) = state.as_ref().filter(|current| current.document == doc) {
            return Ok(current.clone());
        }
        let current = Arc::new(load_serve_state(&self.root, doc)?);
        *state = Some(current.clone());
        Ok(current)
    }

    fn serve_release(&self, stream: &mut TcpStream, path: &str) -> Result<(), String> {
        // Leave capacity for signed documents and heartbeats while bundles stream.
        let blob = if path.starts_with("/v1/blob/") {
            let Some(slot) = Slot::acquire(&self.blobs, MAX_BLOB_DOWNLOADS) else {
                return write_response(
                    stream,
                    503,
                    "text/plain",
                    b"bundle download capacity reached\n",
                );
            };
            Some(slot)
        } else {
            None
        };
        let state = match self.release_state() {
            Ok(state) => state,
            Err(error) => {
                eprintln!("seed release unavailable: {error}");
                return write_response(stream, 503, "text/plain", b"bundle unavailable\n");
            }
        };
        match path {
            "/v1/release" => write_response(stream, 200, "application/json", &state.document),
            "/v1/index" => write_response(stream, 200, "application/json", &state.index),
            _ if blob.is_some() => serve_blob(stream, path, &state),
            _ => write_response(stream, 404, "text/plain", b"not found\n"),
        }
    }
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
    let server = Arc::new(SeedServer {
        root: root.to_path_buf(),
        document_path: document_path.to_path_buf(),
        relay_directory: relay_directory.map(Path::to_path_buf),
        state: Mutex::new(None),
        blobs: Arc::new(AtomicUsize::new(0)),
    });
    let connections = Arc::new(AtomicUsize::new(0));
    let socket = TcpListener::bind(listener)
        .map_err(|error| format!("cannot bind seed listener {listener}: {error}"))?;
    eprintln!("serving bundle seed on {listener}");
    for connection in socket.incoming() {
        let stream = match connection {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("seed listener accept failed: {error}");
                continue;
            }
        };
        if let Err(error) = server.start_request(stream, &connections) {
            eprintln!("cannot start seed request worker: {error}");
        }
    }
    Ok(())
}

fn load_serve_state(root: &Path, document: Vec<u8>) -> Result<ServeState, String> {
    let architecture = local_architecture()?;
    let release = verify_release_document(
        as_utf8(&document, "cached release document")?,
        &architecture,
    )?;
    let current = fs::canonicalize(root.join("current"))
        .map_err(|error| format!("cannot resolve seed release: {error}"))?;
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

fn serve_connection(stream: TcpStream, server: &SeedServer) -> Result<(), String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|error| format!("cannot set seed socket timeouts: {error}"))?;
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader
        .by_ref()
        .take((MAX_HEADERS_BYTES + 1) as u64)
        .read_line(&mut request)
        .map_err(|error| format!("cannot read seed request: {error}"))?;
    if request.len() > MAX_HEADERS_BYTES || !request.ends_with('\n') {
        return Err("invalid seed request line".to_owned());
    }
    let content_length = drain_headers(&mut reader)?;
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let relay_directory = server.relay_directory.as_deref();
    if method == "POST" && path.starts_with("/v1/telemetry/") {
        if content_length > MAX_TELEMETRY_BYTES {
            let mut stream = reader.into_inner();
            return write_response(&mut stream, 413, "text/plain", b"payload too large\n");
        }
        let mut body = vec![0; content_length];
        reader
            .read_exact(&mut body)
            .map_err(|error| format!("cannot read telemetry request body: {error}"))?;
        let mut stream = reader.into_inner();
        return seed_relay::serve_telemetry(&mut stream, path, &body, relay_directory);
    }
    let mut stream = reader.into_inner();
    if method != "GET" {
        return write_response(&mut stream, 405, "text/plain", b"method not allowed\n");
    }
    match path {
        value if value.starts_with("/v1/relay/") => {
            seed_relay::serve_document(&mut stream, value, relay_directory)
        }
        value if value.starts_with("/v1/ping/") => {
            seed_relay::serve_ping(&mut stream, value, relay_directory)
        }
        _ => server.serve_release(&mut stream, path),
    }
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

fn drain_headers(reader: &mut BufReader<TcpStream>) -> Result<usize, String> {
    let mut bytes = 0_usize;
    let mut content_length = None;
    loop {
        let mut line = String::new();
        reader
            .by_ref()
            .take((MAX_HEADERS_BYTES - bytes + 1) as u64)
            .read_line(&mut line)
            .map_err(|error| format!("cannot read seed request headers: {error}"))?;
        bytes += line.len();
        if bytes > MAX_HEADERS_BYTES {
            return Err("seed request headers exceed 8192 bytes".to_owned());
        }
        if line == "\r\n" || line.is_empty() {
            return Ok(content_length.unwrap_or(0));
        }
        if let Some((name, value)) = line.trim().split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            if content_length.is_some() {
                return Err("seed request has duplicate content-length headers".to_owned());
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "seed request has an invalid content-length".to_owned())?,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_config_while_another_connection_stalls_and_no_bundle_is_cached() {
        let root = std::env::temp_dir().join(crate::release::unique_suffix().unwrap());
        let relay = root.join("relay-documents");
        let node = "d6caf939-b959-4471-918f-2e05ca282a37";
        fs::create_dir_all(&relay).unwrap();
        fs::write(relay.join(format!("{node}.json")), b"fixture").unwrap();
        let server = Arc::new(SeedServer {
            root: root.clone(),
            document_path: root.join("missing-release.json"),
            relay_directory: Some(relay),
            state: Mutex::new(None),
            blobs: Arc::default(),
        });
        let slots = Arc::default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let mut stalled = TcpStream::connect(address).unwrap();
        stalled.write_all(b"GET /v1/blob/").unwrap();
        let first = server
            .start_request(listener.accept().unwrap().0, &slots)
            .unwrap()
            .unwrap();

        let mut config = TcpStream::connect(address).unwrap();
        config
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(config, "GET /v1/relay/{node} HTTP/1.1\r\n\r\n").unwrap();
        let second = server
            .start_request(listener.accept().unwrap().0, &slots)
            .unwrap()
            .unwrap();
        let mut response = String::new();
        let result = config.read_to_string(&mut response);
        drop(stalled);
        first.join().unwrap();
        second.join().unwrap();
        fs::remove_dir_all(root).unwrap();

        result.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.ends_with("fixture"));
    }
}
