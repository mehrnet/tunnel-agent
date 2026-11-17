use std::{fs, path::Path};

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
pub use manifest_compile::compile;
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
    let temporary = path.with_extension("next");
    fs::write(&temporary, raw)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))
}

pub fn write_generated_files(plan: &CompiledPlan) -> Result<(), String> {
    for generated in &plan.generated_files {
        let parent = generated.path.parent().expect("generated file has parent");
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        let temporary = generated.path.with_extension("tmp");
        fs::write(&temporary, &generated.content)
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        fs::rename(&temporary, &generated.path)
            .map_err(|error| format!("cannot publish {}: {error}", generated.path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
