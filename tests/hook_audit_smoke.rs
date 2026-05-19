//! `ig hook-audit --json` must emit valid JSON with `drift` + `verdicts_last_n_days`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn json_payload_has_required_keys() {
    let out = Command::new(bin())
        .args(["hook-audit", "--json", "--since", "7"])
        .output()
        .expect("spawn ig hook-audit");
    assert!(
        out.status.success(),
        "ig hook-audit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {} — payload was {:?}", e, stdout));
    assert!(value.get("drift").is_some(), "missing `drift` key");
    assert!(
        value.get("verdicts_last_n_days").is_some(),
        "missing `verdicts_last_n_days` key"
    );
    assert_eq!(value.get("since_days").and_then(|v| v.as_u64()), Some(7));
}

#[test]
fn human_table_runs() {
    let out = Command::new(bin())
        .args(["hook-audit", "--since", "1"])
        .output()
        .expect("spawn ig hook-audit");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Hook integrity"));
    assert!(stdout.contains("Verdict histogram"));
}
