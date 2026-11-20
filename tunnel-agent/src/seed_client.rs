use std::{fs::OpenOptions, path::Path, time::Duration};

use reqwest::{StatusCode, blocking::Client, redirect::Policy};

#[derive(Clone)]
pub(crate) struct SeedSource {
    base: String,
    client: Client,
}

impl SeedSource {
    pub(crate) fn new(base: &str) -> Result<Self, String> {
        Self::build(base, None)
    }

    pub(crate) fn with_timeout(base: &str, timeout: Duration) -> Result<Self, String> {
        Self::build(base, Some(timeout))
    }

    fn build(base: &str, timeout: Option<Duration>) -> Result<Self, String> {
        let url = reqwest::Url::parse(base).map_err(|_| "seed source URL is invalid".to_owned())?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1") | Some("::1"))
            || url.port().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("seed source must be an explicit loopback HTTP listener".to_owned());
        }
        let client = Client::builder()
            .redirect(Policy::none())
            // Seed traffic is confined to the explicitly mapped management lane.
            .no_proxy();
        let client = if let Some(timeout) = timeout {
            client.timeout(timeout)
        } else {
            client
        }
        .build()
        .map_err(|error| format!("cannot build seed client: {error}"))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            client,
        })
    }

    pub(crate) fn relay_document(&self, node_id: &str) -> Result<String, String> {
        if !crate::identity::valid_uuid(node_id) {
            return Err("relay node id is invalid".to_owned());
        }
        let raw = self.get_bytes(
            &format!("/v1/relay/{node_id}"),
            crate::seed::MAX_RELAY_DOCUMENT_BYTES,
        )?;
        String::from_utf8(raw).map_err(|_| "relay response is not UTF-8".to_owned())
    }

    pub(crate) fn get_bytes(&self, path: &str, limit: u64) -> Result<Vec<u8>, String> {
        let response = self.request(path)?;
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err("seed response exceeds the maximum size".to_owned());
        }
        let body = response
            .bytes()
            .map_err(|error| format!("cannot read seed response: {error}"))?;
        if body.len() as u64 > limit {
            return Err("seed response exceeds the maximum size".to_owned());
        }
        Ok(body.to_vec())
    }

    pub(crate) fn get_file(
        &self,
        path: &str,
        destination: &Path,
        expected_bytes: u64,
    ) -> Result<(), String> {
        let mut response = self.request(path)?;
        if response.content_length() != Some(expected_bytes) {
            return Err("seed blob length does not match its inventory".to_owned());
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .map_err(|error| format!("cannot create seed blob cache: {error}"))?;
        let copied = response
            .copy_to(&mut file)
            .map_err(|error| format!("cannot read seed blob: {error}"))?;
        if copied == expected_bytes {
            Ok(())
        } else {
            Err("seed blob length does not match its inventory".to_owned())
        }
    }

    fn request(&self, path: &str) -> Result<reqwest::blocking::Response, String> {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .send()
            .map_err(|error| format!("cannot contact seed source: {error}"))?;
        if response.status() == StatusCode::OK {
            Ok(response)
        } else {
            Err(format!("seed source returned HTTP {}", response.status()))
        }
    }
}
