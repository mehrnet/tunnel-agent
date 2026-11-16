use crate::{
    agent::{AgentSettings, read_secret_file},
    manifest::Config,
    seed,
};

pub(crate) async fn send(settings: &AgentSettings, config: &Config) -> Result<(), String> {
    let Some(source) = config
        .agent
        .bundle_seed
        .as_ref()
        .map(|seed| seed.source.clone())
    else {
        return Ok(());
    };
    let node_id_file = settings.node_id_file.clone();
    tokio::task::spawn_blocking(move || {
        let node_id = read_secret_file(&node_id_file, "node id")?;
        seed::ping_relay(&source, &node_id)
    })
    .await
    .map_err(|error| format!("relay heartbeat task failed: {error}"))?
}
