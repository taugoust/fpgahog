use serde_json::Value;
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn lease_cli_is_machine_readable_and_isolated_from_host_state() {
    let root = std::env::temp_dir().join(format!(
        "hosthog-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("leases.json");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fpgahog"))
            .args(args)
            .env("HOSTHOG_LEASE_STATE", &path)
            .output()
            .unwrap()
    };
    let acquired = run(&[
        "lease",
        "acquire",
        "json-test-resource",
        "--mode",
        "exclusive",
        "--session",
        "cli-test",
    ]);
    assert!(acquired.status.success());
    assert!(acquired.stderr.is_empty());
    let body: Value = serde_json::from_slice(&acquired.stdout).unwrap();
    assert_eq!(body["version"], 1);
    let token = body["result"]["lease"]["token"].as_str().unwrap();
    let busy = run(&["lease", "acquire", "json-test-resource", "--mode", "shared"]);
    assert_eq!(busy.status.code(), Some(3));
    assert!(busy.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&busy.stderr).unwrap()["error"],
        "busy"
    );
    let released = run(&["lease", "release", "json-test-resource", "--token", token]);
    assert!(released.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&released.stdout).unwrap()["version"],
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
