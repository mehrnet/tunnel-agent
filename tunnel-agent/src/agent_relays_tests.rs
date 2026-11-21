use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};

use super::*;
use crate::manifest::{self, Config};

pub(crate) fn lane(transport: &str, id: char, control: u16, listener: u16, role: &str) -> Value {
    let server = role == "server";
    let mut lane = json!({
        "id": id.to_string().repeat(17),
        "transport": transport, "direction": "foreign_to_domestic", "role": role,
        "priority": control,
        "peer": {"node_id": "d6caf939-b959-4471-918f-2e05ca282a37", "host": "127.0.0.1", "port": control},
        "listener": {"bind": "127.0.0.1", "port": listener},
    });
    let target = json!({"host": "127.0.0.1", "port": 42100});
    lane[transport] = match transport {
        "hpnssh" => {
            let fields = if server {
                json!({"bind":"0.0.0.0","port":control,"host_key_file":"/test/host","authorized_keys_file":"/test/authorized_keys"})
            } else {
                json!({"user":"root","identity_file":"/test/id","known_hosts_file":"/test/known_hosts",
                    "known_host":format!("[127.0.0.1]:{control} ssh-ed25519 AAAA"),"target":target})
            };
            json!({"cluster_id": "4cd57b8f-8f97-4f80-8791-90bba10190d0", role: fields})
        }
        "wstunnel" => json!({
            "certificate_file":"/test/pair.crt", "key_file":"/test/pair.key", "peer_ca_file":"/test/peer-ca.pem",
            "tls_name":(!server).then_some("management.example"),
            "target":(!server).then_some(target)
        }),
        "rathole" => json!({
            "local_key": "Jc7TZcV1b5sOGer+PJkD/b7c3ESqseSBWqlOiK6Dl1E=",
            "peer_key": "C3mqkz7RYe6F34129eHGW14m5PFy70hU5A/QYRbeRb4=",
            "generation": "1".repeat(64),
            "target":(!server).then_some(target)
        }),
        _ => unreachable!(),
    };
    lane
}

pub(crate) fn config(scope: &str, bundle: &Path, active: usize, lanes: Vec<Value>) -> String {
    json!({"version":3,"node_scope":scope,"bundle":{"root":bundle},"agent":{
        "max_active_lanes":active,"lanes":lanes,
    }})
    .to_string()
}

fn mixed(scope: &str, role: &str, active: usize) -> String {
    config(
        scope,
        Path::new("/opt/tunnel-agent/current"),
        active,
        vec![
            lane("rathole", 'R', 42003, 42033, role),
            lane("hpnssh", 'H', 42001, 42031, role),
            lane("wstunnel", 'W', 42002, 42032, role),
        ],
    )
}

#[test]
fn plans_mixed_transports_and_deduplicates_the_foreign_seed() {
    let foreign = manifest::parse_config(&mixed("foreign", "client", 3)).unwrap();
    let domestic = manifest::parse_config(&mixed("domestic", "server", 2)).unwrap();
    let state = Path::new("/test");
    let plan = manifest::validate_plan(&foreign, state).unwrap();
    assert_eq!(plan.processes.len(), 4);
    assert_eq!(plan.listeners.len(), 1);
    assert_eq!(plan.listeners[0].port, 42100);
    let server_plan = manifest::validate_plan(&domestic, state).unwrap();
    assert_eq!(server_plan.processes.len(), 2);
    assert_eq!(server_plan.listeners.len(), 4);
    assert_eq!(
        server_plan
            .agent_lanes
            .iter()
            .filter(|lane| lane.selected)
            .count(),
        2
    );
    assert_eq!(
        sources(&domestic.agent),
        ["http://127.0.0.1:42031", "http://127.0.0.1:42032"]
    );
}

#[test]
fn rejects_public_relay_targets_and_overlapping_active_ports() {
    let mut invalid: Value = serde_json::from_str(&mixed("foreign", "client", 3)).unwrap();
    invalid["agent"]["lanes"][0]["rathole"]["target"]["host"] = json!("192.0.2.42");
    let invalid: Config = serde_json::from_value(invalid).unwrap();
    assert!(manifest::validate_plan(&invalid, Path::new("/test")).is_err());

    let mut collision: Value = serde_json::from_str(&mixed("domestic", "server", 3)).unwrap();
    collision["agent"]["lanes"][0]["listener"]["port"] = json!(42032);
    let collision = serde_json::from_value(collision).unwrap();
    assert!(
        manifest::validate_plan(&collision, Path::new("/test"))
            .unwrap_err()
            .contains("conflict")
    );
}

#[test]
fn picks_newest_reachable_revision_and_falls_back_for_writes() {
    let sources = vec![
        "unavailable".to_owned(),
        "stale".to_owned(),
        "fresh".to_owned(),
    ];
    let attempts = AtomicUsize::new(0);
    let requested = collect(&sources, |source| match source {
        "stale" => Ok(RemoteDocument {
            revision: 10,
            manifest_json: "{}".to_owned(),
        }),
        "fresh" => Ok(RemoteDocument {
            revision: 12,
            manifest_json: "{}".to_owned(),
        }),
        _ => Err("connection closed".to_owned()),
    });
    assert_eq!(newest(requested).unwrap().revision, 12);
    let used = first_working(&sources, |source| {
        attempts.fetch_add(1, Ordering::Relaxed);
        if source == "unavailable" {
            Err("connection closed".to_owned())
        } else {
            Ok(source.to_owned())
        }
    })
    .unwrap();
    assert_eq!(used, "stale");
    assert_eq!(attempts.into_inner(), 2);
}

#[test]
fn legacy_seed_remains_available_without_duplicate_requests() {
    let mut manifest: Value = serde_json::from_str(&mixed("domestic", "server", 2)).unwrap();
    manifest["agent"]["bundle_seed"] = json!({"source": "http://127.0.0.1:42031"});
    let parsed = crate::manifest::parse_config(&manifest.to_string()).unwrap();
    assert_eq!(sources(&parsed.agent).len(), 2);
    manifest["agent"]["lanes"] = json!([]);
    let parsed = crate::manifest::parse_config(&manifest.to_string()).unwrap();
    assert_eq!(sources(&parsed.agent), ["http://127.0.0.1:42031"]);
}

#[test]
fn stale_noise_identity_does_not_stop_other_management_lanes_at_boot() {
    let mut domestic = manifest::parse_config(&mixed("domestic", "server", 3)).unwrap();
    domestic
        .agent
        .retain_startable_lanes(&crate::transport_identity::TransportKeys::default());
    assert_eq!(
        sources(&domestic.agent),
        ["http://127.0.0.1:42031", "http://127.0.0.1:42032"]
    );
}

#[test]
fn admits_eighteen_parallel_management_paths_for_a_six_peer_mesh() {
    let lanes = (0..18_u16)
        .map(|index| {
            let transport = ["hpnssh", "wstunnel", "rathole"][usize::from(index % 3)];
            lane(
                transport,
                char::from(b'A' + index as u8),
                42000 + index,
                42100 + index,
                "server",
            )
        })
        .collect();
    let raw = config(
        "domestic",
        Path::new("/opt/tunnel-agent/current"),
        18,
        lanes,
    );
    let domestic = manifest::parse_config(&raw).unwrap();
    let plan = manifest::validate_plan(&domestic, Path::new("/test")).unwrap();
    assert_eq!(plan.processes.len(), 18);
    assert_eq!(plan.listeners.len(), 36);
    assert_eq!(sources(&domestic.agent).len(), 18);
}
