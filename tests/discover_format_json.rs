//! `ig discover --format json` must emit schema-stable JSON.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn discover_json_has_stable_schema() {
    let out = Command::new(bin())
        .args(["discover", "--format", "json", "--since", "1"])
        .output()
        .expect("spawn ig discover");
    assert!(
        out.status.success(),
        "ig discover --format json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {} — payload was {:?}", e, stdout));

    for key in [
        "scanned_commands",
        "rewritable",
        "missed",
        "rtk_disabled",
        "agent_integration_status",
    ] {
        assert!(value.get(key).is_some(), "missing `{key}` key");
    }
    assert!(value["missed"].is_array(), "`missed` must be an array");
    assert!(
        value["rtk_disabled"].is_array(),
        "`rtk_disabled` must be an array"
    );
    assert!(
        value["agent_integration_status"].is_object(),
        "`agent_integration_status` must be an object"
    );
}

#[test]
fn discover_json_all_flag_accepted() {
    let out = Command::new(bin())
        .args(["discover", "--format", "json", "--all"])
        .output()
        .expect("spawn ig discover --all");
    assert!(out.status.success(), "ig discover --all failed");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
        "ig discover --all --format json did not emit valid JSON"
    );
}
