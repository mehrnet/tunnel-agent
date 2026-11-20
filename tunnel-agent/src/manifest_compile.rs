use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{agent_lane, agent_spec};

use super::{
    CompiledPlan,
    manifest_compile_tunnels::compile_tunnels,
    manifest_schema::{BUNDLED_ARTIFACTS, Config},
    manifest_validate::{validate_config, validate_listener_conflicts, validate_processes},
};

pub fn compile(config: &Config, state_dir: &Path) -> Result<CompiledPlan, String> {
    compile_plan(config, state_dir, false)
}

pub fn validate_plan(config: &Config, state_dir: &Path) -> Result<CompiledPlan, String> {
    compile_plan(config, state_dir, true)
}

fn compile_plan(
    config: &Config,
    state_dir: &Path,
    check_only: bool,
) -> Result<CompiledPlan, String> {
    validate_config(config)?;
    let artifact_paths = resolve_artifact_paths(config, state_dir)?;
    let compilation = compile_tunnels(config, state_dir, &artifact_paths, check_only)?;
    let selected = agent_spec::selected_lanes(&config.agent);
    let selected_ids: HashSet<_> = selected
        .iter()
        .filter(|lane| lane.selected)
        .map(|lane| lane.id.as_str())
        .collect();
    let mut processes = compilation.processes;
    let mut generated_files = compilation.generated_files;
    let mut listeners = compilation.listeners;
    let mut agent_lanes = Vec::new();

    for lane in config.agent.lanes.iter().filter(|lane| lane.enabled) {
        let bundle_root = config
            .bundle
            .as_ref()
            .ok_or_else(|| "agent lanes require a bundled manifest".to_owned())?
            .root
            .as_path();
        let (plan, lane_processes, lane_files, lane_listeners) = agent_lane::compile(
            lane,
            selected_ids.contains(lane.id.as_str()),
            bundle_root,
            state_dir,
        );
        processes.extend(lane_processes);
        generated_files.extend(lane_files);
        listeners.extend(lane_listeners);
        agent_lanes.push(plan);
    }

    crate::ingress::compile(
        config,
        state_dir,
        &mut processes,
        &mut generated_files,
        &mut listeners,
    )?;
    crate::probe_job::compile(config, state_dir, &mut processes, &mut generated_files)?;
    validate_processes(&processes)?;
    validate_listener_conflicts(&listeners)?;
    Ok(CompiledPlan {
        processes,
        generated_files,
        listeners,
        agent_lanes,
    })
}

fn resolve_artifact_paths(
    config: &Config,
    state_dir: &Path,
) -> Result<HashMap<String, PathBuf>, String> {
    if let Some(bundle) = &config.bundle {
        return BUNDLED_ARTIFACTS
            .iter()
            .map(|(id, relative)| Ok(((*id).to_owned(), bundle.root.join(relative))))
            .collect();
    }
    Ok(config
        .artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.id.clone(),
                super::artifact_binary_path(state_dir, artifact),
            )
        })
        .collect())
}
