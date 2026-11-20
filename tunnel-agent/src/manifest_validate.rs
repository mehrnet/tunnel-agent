use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path},
};

use crate::{agent_lane, agent_spec};

use super::{
    manifest_schema::{
        BUNDLED_ARTIFACTS, BundleSpec, Config, NodeScope, ProcessSpec, RestartPolicy,
    },
    manifest_validate_tunnels::validate_tunnel,
};

pub fn validate_config(config: &Config) -> Result<(), String> {
    match config.version {
        1 => validate_version_one(config)?,
        2 => validate_version_two(config)?,
        3 => validate_version_three(config)?,
        version => {
            return Err(format!(
                "unsupported manifest version {version}; expected 1, 2, or 3"
            ));
        }
    }

    validate_processes(&config.processes)?;
    validate_artifacts(&config.artifacts)?;
    if let Some(ingress) = &config.public_ingress {
        if config.version != 3 {
            return Err("public ingress requires version 3".to_owned());
        }
        crate::ingress::valid(ingress)?;
    }
    validate_tunnels(config)
}

fn validate_version_one(config: &Config) -> Result<(), String> {
    if config.bundle.is_some()
        || !config.artifacts.is_empty()
        || !config.tunnels.is_empty()
        || !config.agent.is_default()
    {
        return Err("manifest version 1 only supports processes".to_owned());
    }
    if config.processes.is_empty() {
        return Err("processes must not be empty".to_owned());
    }
    Ok(())
}

fn validate_version_two(config: &Config) -> Result<(), String> {
    if config.bundle.is_some() {
        return Err("manifest version 2 must not define bundle".to_owned());
    }
    if !config.agent.is_default() {
        return Err("manifest version 2 must not define agent settings".to_owned());
    }
    if config.processes.is_empty() && config.tunnels.is_empty() {
        return Err("version 2 requires at least one process or tunnel".to_owned());
    }
    Ok(())
}

fn validate_version_three(config: &Config) -> Result<(), String> {
    if !config.artifacts.is_empty() {
        return Err(
            "manifest version 3 uses the installed bundle and must not define artifacts".to_owned(),
        );
    }
    if !config.processes.is_empty() {
        return Err(
            "manifest version 3 supports typed tunnels only; raw processes are not allowed"
                .to_owned(),
        );
    }
    validate_bundle(
        config
            .bundle
            .as_ref()
            .ok_or_else(|| "manifest version 3 requires bundle.root".to_owned())?,
    )?;
    agent_spec::validate(&config.agent)?;
    for lane in config.agent.lanes.iter().filter(|lane| lane.enabled) {
        agent_lane::validate(lane, config.node_scope)?;
    }
    if config.agent.bundle_seed.is_some() && config.node_scope != NodeScope::Domestic {
        return Err("agent.bundle_seed is supported only on domestic nodes".to_owned());
    }
    Ok(())
}

fn validate_tunnels(config: &Config) -> Result<(), String> {
    let artifacts: HashSet<&str> = match &config.bundle {
        Some(_) => BUNDLED_ARTIFACTS.iter().map(|(id, _)| *id).collect(),
        None => config
            .artifacts
            .iter()
            .map(|artifact| artifact.id.as_str())
            .collect(),
    };
    let mut ids: HashSet<&str> = config
        .processes
        .iter()
        .map(|process| process.id.as_str())
        .collect();
    for tunnel in &config.tunnels {
        validate_id(tunnel.id(), "tunnel")?;
        if !ids.insert(tunnel.id()) {
            return Err(format!("duplicate process or tunnel id {:?}", tunnel.id()));
        }
        if tunnel.enabled() {
            validate_tunnel(tunnel, &artifacts)?;
        }
    }
    Ok(())
}

fn validate_bundle(bundle: &BundleSpec) -> Result<(), String> {
    if !bundle.root.is_absolute() {
        return Err("bundle.root must be an absolute path".to_owned());
    }
    if bundle.root.as_os_str().is_empty() {
        return Err("bundle.root must not be empty".to_owned());
    }
    Ok(())
}

pub(super) fn validate_processes(processes: &[ProcessSpec]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for process in processes {
        validate_id(&process.id, "process")?;
        if !ids.insert(process.id.as_str()) {
            return Err(format!("duplicate process id {:?}", process.id));
        }
        if process.argv.is_empty() || process.argv[0].trim().is_empty() {
            return Err(format!(
                "process {:?} must contain a non-empty argv[0]",
                process.id
            ));
        }
        validate_restart(&process.id, &process.restart)?;
        if process
            .env
            .keys()
            .any(|key| key.is_empty() || key.contains('='))
        {
            return Err(format!(
                "process {:?} has an invalid environment key",
                process.id
            ));
        }
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[super::ArtifactSpec]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for artifact in artifacts {
        validate_id(&artifact.id, "artifact")?;
        if !ids.insert(artifact.id.as_str()) {
            return Err(format!("duplicate artifact id {:?}", artifact.id));
        }
        validate_safe_component(&artifact.version, "artifact version")?;
        validate_relative_path(&artifact.binary, "artifact binary")?;
        validate_sha256(&artifact.sha256, "sha256", &artifact.id)?;
        if let Some(binary_sha256) = &artifact.binary_sha256 {
            validate_sha256(binary_sha256, "binary_sha256", &artifact.id)?;
        }
        if artifact.sources.is_empty() {
            return Err(format!(
                "artifact {:?} requires at least one source",
                artifact.id
            ));
        }
        for source in &artifact.sources {
            if !source.url.starts_with("https://") {
                return Err(format!("artifact {:?} source must use HTTPS", artifact.id));
            }
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, field: &str, artifact_id: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "artifact {artifact_id:?} {field} must be 64 hexadecimal characters"
        ));
    }
    Ok(())
}

pub(super) fn validate_id(id: &str, subject: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "{subject} id {:?} must use only letters, numbers, '-' or '_'",
            id
        ));
    }
    Ok(())
}

fn validate_safe_component(value: &str, subject: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(format!(
            "{subject} {:?} is not a safe path component",
            value
        ));
    }
    Ok(())
}

fn validate_relative_path(path: &Path, subject: &str) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{subject} must be a non-empty relative path without traversal"
        ));
    }
    Ok(())
}

pub(super) fn validate_restart(id: &str, restart: &RestartPolicy) -> Result<(), String> {
    if restart.initial_backoff_ms == 0 {
        return Err(format!(
            "process {id:?} initial_backoff_ms must be positive"
        ));
    }
    if restart.max_backoff_ms < restart.initial_backoff_ms {
        return Err(format!(
            "process {id:?} max_backoff_ms must be at least initial_backoff_ms"
        ));
    }
    if restart.reset_after_secs == 0 {
        return Err(format!("process {id:?} reset_after_secs must be positive"));
    }
    Ok(())
}

pub(super) fn validate_listener_conflicts(
    listeners: &[super::ListenerClaim],
) -> Result<(), String> {
    let mut claimed = HashMap::new();
    for listener in listeners {
        let key = (&listener.bind, listener.port);
        if let Some(previous) = claimed.insert(key, &listener.owner) {
            return Err(format!(
                "local listener conflict: {previous:?} and {:?} both claim {}:{}",
                listener.owner, listener.bind, listener.port
            ));
        }
    }
    Ok(())
}
