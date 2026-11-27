#[test]
fn ingress_cli_dispatches_without_entering_tokio() {
    let missing_config = std::env::temp_dir().join(format!(
        "tunnel-agent-missing-ingress-{}",
        std::process::id()
    ));
    assert!(!missing_config.exists());
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_tunnel-agent"))
        .args(["public-ingress-worker", "--config"])
        .arg(missing_config)
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(error.contains("public ingress worker failed"), "{error}");
    assert!(
        !error.contains("must run outside the Tokio runtime"),
        "{error}"
    );
}
