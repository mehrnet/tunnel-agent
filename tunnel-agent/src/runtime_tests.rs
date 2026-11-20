use super::*;
use crate::{manifest::RestartPolicy, release::unique_suffix};
use std::{collections::BTreeMap, fs};

fn worker(root: &Path, id: &str, arg: &str, config: Option<&Path>) -> ProcessSpec {
    let mut argv = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        "printf '%s\n' \"$$\" >\"$1\"; exec sleep 120".to_owned(),
        "test-worker".to_owned(),
        root.join(format!("{id}.pid")).display().to_string(),
        arg.to_owned(),
    ];
    if let Some(config) = config {
        argv.push(format!("config={}", config.display()));
    }
    ProcessSpec {
        id: id.to_owned(),
        argv,
        env: BTreeMap::new(),
        workdir: None,
        restart: RestartPolicy::default(),
    }
}

fn plan(processes: Vec<ProcessSpec>, generated_files: Vec<GeneratedFile>) -> CompiledPlan {
    CompiledPlan {
        processes,
        generated_files,
        listeners: vec![],
        agent_lanes: vec![],
    }
}

fn pid_file(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.pid"))
}

async fn started(root: &Path, id: &str, previous: Option<i32>) -> i32 {
    let path = pid_file(root, id);
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = fs::read_to_string(&path).unwrap_or_default().trim().parse()
                && Some(pid) != previous
            {
                return pid;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker started")
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn phase_plans(root: &Path, phase: &str, management: &ProcessSpec) -> (CompiledPlan, CompiledPlan) {
    let raw = fs::read_to_string(root.join(format!("{phase}.json"))).unwrap();
    let mut pair: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let compile = |manifest_json: &mut serde_json::Value, side| {
        manifest_json["bundle"]["root"] = serde_json::json!(root);
        if side == "server"
            && let Some(tunnel) = manifest_json["tunnels"].get_mut(0)
        {
            tunnel["tls_certificate"] = serde_json::json!(root.join("cert.pem"));
            tunnel["tls_private_key"] = serde_json::json!(root.join("key.pem"));
            tunnel["listener"]["bind"] = serde_json::json!("127.0.0.1");
        }
        let config = manifest::parse_config(&manifest_json.to_string()).unwrap();
        let mut plan = manifest::compile(&config, &root.join(side)).unwrap();
        if side == "server" {
            plan.processes.push(management.clone());
        }
        plan
    };
    let server = compile(&mut pair["server"]["manifest"], "server");
    let client = compile(&mut pair["client"]["manifest"], "client");
    (server, client)
}

async fn probe() -> Option<String> {
    let response = Command::new("curl")
        .args([
            "--noproxy",
            "*",
            "--silent",
            "--fail",
            "--max-time",
            "2",
            "http://127.0.0.1:31402/check",
        ])
        .output()
        .await
        .unwrap();
    response
        .status
        .success()
        .then(|| String::from_utf8(response.stdout).unwrap())
}

#[tokio::test]
#[ignore = "needs Worker-generated fixtures, wstunnel, TLS, and loopback HTTP fixtures"]
async fn worker_lifecycle_over_live_wstunnel() {
    let root = PathBuf::from(std::env::var_os("TUNNEL_TEST_MANIFEST_DIR").unwrap());
    let management = worker(&root, "agent-lane-control", "sentinel", None);
    let (server, client) = phase_plans(&root, "draft", &management);
    let mut listener = start_plan(server).await.unwrap();
    let mut connector = start_plan(client).await.unwrap();
    let management_pid = started(&root, &management.id, None).await;
    assert_eq!(probe().await, None, "draft must not listen");

    for (phase, expected) in [
        ("enabled", "primary"),
        ("restarted", "primary"),
        ("edited", "secondary"),
    ] {
        let (server, client) = phase_plans(&root, phase, &management);
        reconcile_plan(&mut listener, server, false).await;
        reconcile_plan(&mut connector, client, false).await;
        timeout(Duration::from_secs(20), async {
            loop {
                if probe().await.as_deref() == Some(expected) {
                    return;
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no {expected} response after {phase}"));
        assert!(
            alive(management_pid),
            "management PID changed after {phase}"
        );
    }
    for phase in ["disabled", "removed"] {
        let (server, client) = phase_plans(&root, phase, &management);
        reconcile_plan(&mut connector, client, false).await;
        reconcile_plan(&mut listener, server, false).await;
        assert_eq!(probe().await, None, "traffic continued after {phase}");
        assert!(
            alive(management_pid),
            "management PID changed after {phase}"
        );
    }
    stop_plan(&mut connector).await;
    stop_plan(&mut listener).await;
}

#[tokio::test]
async fn lane_lifecycle_keeps_management_and_unrelated_workers_running() {
    let root = std::env::temp_dir().join(unique_suffix().unwrap());
    fs::create_dir(&root).unwrap();
    let management = worker(&root, "agent-lane-control", "management", None);
    let seed = worker(&root, "agent-seed-control", "seed", None);
    let other = worker(&root, "existing", "unrelated", None);
    let base = vec![management.clone(), seed.clone(), other.clone()];
    let mut running = start_plan(plan(base.clone(), vec![])).await.unwrap();
    let management_pid = started(&root, &management.id, None).await;
    let seed_pid = started(&root, &seed.id, None).await;
    let other_pid = started(&root, &other.id, None).await;
    let config_path = root.join("owned.toml");
    let lane = worker(&root, "owned-test", "initial", Some(&config_path));
    let updated = worker(&root, "owned-test", "new destination", Some(&config_path));
    let mut all = base.clone();
    all.push(lane.clone());
    let mut generated = GeneratedFile {
        path: config_path,
        content: "fixture".to_owned(),
    };
    let next = plan(all.clone(), vec![generated.clone()]);
    manifest::write_generated_files(&next).unwrap();
    reconcile_plan(&mut running, next, false).await;
    let initial_pid = started(&root, &lane.id, None).await;

    all[3] = updated;
    reconcile_plan(
        &mut running,
        plan(all.clone(), vec![generated.clone()]),
        false,
    )
    .await;
    let edited_pid = started(&root, &lane.id, Some(initial_pid)).await;
    assert!(!alive(initial_pid));

    generated.content = "new-generation".to_owned();
    let next = plan(all, vec![generated]);
    manifest::write_generated_files(&next).unwrap();
    reconcile_plan(&mut running, next, false).await;
    let restarted_pid = started(&root, &lane.id, Some(edited_pid)).await;
    assert!(!alive(edited_pid));

    reconcile_plan(&mut running, plan(base.clone(), vec![]), false).await;
    assert!(!alive(restarted_pid));
    reconcile_plan(&mut running, plan(base, vec![]), false).await;
    for (id, pid) in [
        (management.id, management_pid),
        (seed.id, seed_pid),
        (other.id, other_pid),
    ] {
        assert!(alive(pid));
        assert_eq!(
            fs::read_to_string(pid_file(&root, &id)).unwrap().trim(),
            pid.to_string()
        );
    }
    stop_plan(&mut running).await;
    fs::remove_dir_all(root).unwrap();
}
