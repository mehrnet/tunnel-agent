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
