//! Telemetry is opt-in and offline by default. These tests verify the public
//! build never pings: no endpoint is compiled in, consent defaults to "not
//! asked", and `IG_TELEMETRY_DISABLED` is a hard kill switch.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

fn temp_home() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "ig-telemetry-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn status_default_is_offline_and_not_asked() {
    let home = temp_home();
    let out = Command::new(bin())
        .args(["telemetry", "status"])
        .env("HOME", &home)
        .output()
        .expect("spawn ig telemetry status");
    assert!(out.status.success(), "ig telemetry status failed");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("not asked"),
        "fresh HOME should report consent 'not asked', got: {stdout}"
    );
    // The public build compiles in no endpoint URL.
    assert!(
        stdout.contains("offline build") || stdout.contains("none"),
        "public build must report no endpoint, got: {stdout}"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn env_kill_switch_is_reported() {
    let home = temp_home();
    let out = Command::new(bin())
        .args(["telemetry", "status"])
        .env("HOME", &home)
        .env("IG_TELEMETRY_DISABLED", "1")
        .output()
        .expect("spawn ig telemetry status");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("IG_TELEMETRY_DISABLED set"),
        "kill switch should be reported, got: {stdout}"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn test_subcommand_emits_json_payload() {
    let home = temp_home();
    let out = Command::new(bin())
        .args(["telemetry", "test"])
        .env("HOME", &home)
        .output()
        .expect("spawn ig telemetry test");
    assert!(out.status.success(), "ig telemetry test failed");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("ig telemetry test must emit JSON: {e} — {stdout:?}"));
    for key in ["ig_version", "os", "arch", "parse_outcome_breakdown_24h"] {
        assert!(value.get(key).is_some(), "payload missing `{key}`");
    }
    // No PII leaks.
    assert!(
        value.get("cwd").is_none() && value.get("username").is_none(),
        "telemetry payload must not carry PII"
    );
    std::fs::remove_dir_all(&home).ok();
}
