//! `ig setup --show --agent claude` after a clean install must list every
//! managed artifact.

use std::fs;
use std::process::Command;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn show_lists_installed_claude_artifacts() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();

    // Install first.
    Command::new(ig_bin())
        .args(["setup", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .unwrap();

    let out = Command::new(ig_bin())
        .args(["setup", "--show", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig setup --show ran");
    assert!(out.status.success(), "show failed");
    let s = String::from_utf8_lossy(&out.stderr);
    assert!(s.contains("Claude Code"), "show header missing: {}", s);
    assert!(s.contains("CLAUDE.md"), "show should list CLAUDE.md");
    assert!(s.contains("ig-guard.sh"), "show should list ig-guard.sh");
    assert!(
        s.contains("settings.json"),
        "show should list settings.json"
    );
}
