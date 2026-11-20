use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

#[path = "manifest_artifact.rs"]
mod manifest_artifact;
#[path = "manifest_compile.rs"]
mod manifest_compile;
#[path = "manifest_compile_tunnels.rs"]
mod manifest_compile_tunnels;
#[path = "manifest_schema.rs"]
mod manifest_schema;
#[path = "manifest_validate.rs"]
mod manifest_validate;
#[path = "manifest_validate_tunnels.rs"]
mod manifest_validate_tunnels;

pub use manifest_artifact::sync_artifacts;
pub use manifest_compile::{compile, validate_plan};
pub use manifest_schema::*;
pub use manifest_validate::validate_config;

#[cfg(test)]
use std::{collections::BTreeMap, path::PathBuf};

#[cfg(test)]
use crate::agent_spec::AgentSpec;

#[cfg(test)]
use manifest_artifact::{install_download, sha256_file};

pub fn load_config(path: &Path) -> Result<Config, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse_config(&raw)
}

pub fn artifact_binary_path(state_dir: &Path, artifact: &ArtifactSpec) -> std::path::PathBuf {
    manifest_artifact::artifact_binary_path(state_dir, artifact)
}

pub fn parse_config(raw: &str) -> Result<Config, String> {
    let config: Config = serde_json::from_str(raw)
        .map_err(|error| format!("cannot parse configuration: {error}"))?;
    validate_config(&config)?;
    Ok(config)
}

pub fn write_config_atomic(path: &Path, raw: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("configuration path {} has no parent", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension(format!("next-{}", crate::release::unique_suffix()?));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
        file.write_all(raw.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot publish {}: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn write_generated_files(plan: &CompiledPlan) -> Result<(), String> {
    for generated in &plan.generated_files {
        let protected = fs::symlink_metadata(&generated.path).is_ok_and(|metadata| {
            metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600
        });
        if protected
            && fs::read_to_string(&generated.path).is_ok_and(|value| value == generated.content)
        {
            continue;
        }
        write_config_atomic(&generated.path, &generated.content)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
