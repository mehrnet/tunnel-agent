use super::*;

const NODE_ID: &str = "d6caf939-b959-4471-918f-2e05ca282a37";

fn bundled_config(scope: &str, root: &str) -> Config {
    manifest::parse_config(&format!(
        r#"{{
            "version": 3,
            "node_scope": "{scope}",
            "bundle": {{ "root": "{root}" }},
            "tunnels": []
        }}"#
    ))
    .expect("valid v3 test manifest")
}

#[test]
fn accepts_node_token_bound_to_its_node_id() {
    validate_node_credentials(
        NODE_ID,
        "d6caf939-b959-4471-918f-2e05ca282a37.0123456789abcdef",
    )
    .expect("token is bound to node id");
}

#[test]
fn rejects_node_token_for_another_node() {
    assert!(
        validate_node_credentials(
            NODE_ID,
            "43d4d9e2-9131-4e84-bcd5-f2baac3f0214.0123456789abcdef",
        )
        .is_err()
    );
}

#[test]
fn remote_document_cannot_change_scope_or_bundle_root() {
    let active = bundled_config("domestic", "/opt/tunnel-agent/current");
    let changed_scope = bundled_config("foreign", "/opt/tunnel-agent/current");
    let changed_root = bundled_config("domestic", "/opt/tunnel-agent/other");
    assert!(ensure_compatible_document(&active, &changed_scope).is_err());
    assert!(ensure_compatible_document(&active, &changed_root).is_err());
}

#[test]
fn legacy_manifest_defaults_to_independent_poll_periods() {
    let mut config = bundled_config("domestic", "/opt/tunnel-agent/current");
    config.agent.bundle_seed = Some(
        serde_json::from_value(serde_json::json!({
            "source": "http://127.0.0.1:42022",
        }))
        .unwrap(),
    );
    assert_eq!(config.agent.config_poll_interval_secs, 5);
    assert_eq!(config.agent.poll_interval_secs, 60);
    assert_eq!(offline_poll_interval(&config), 300);

    let before = config.agent.clone();
    config.agent.config_poll_interval_secs = 10;
    assert!(!before.lanes_changed(&config.agent));
}

#[tokio::test]
async fn a_busy_bundle_job_does_not_queue_duplicates_or_block_config_polling() {
    let (release, wait) = std::sync::mpsc::channel();
    let mut bundle = PollJob::new();
    bundle.start(move || {
        wait.recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())
    });
    bundle.start(|| panic!("must not start a second update while one is in flight"));
    let mut config = PollJob::new();
    config.start(|| Ok(42));
    let revision = tokio::time::timeout(Duration::from_secs(1), config.result()).await;
    let _ = release.send(());
    assert_eq!(revision.unwrap().unwrap(), 42);
    bundle.result().await.unwrap();
}
