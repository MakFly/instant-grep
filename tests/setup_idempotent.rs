//! `ig setup --agent claude` run twice — second invocation must be a no-op
//! (zero configured items per the InstallReport).

use std::fs;
use std::process::Command;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn second_run_is_no_op_for_claude() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();

    // first install — should configure several items
    let first = Command::new(ig_bin())
        .args(["setup", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig setup ran");
    assert!(first.status.success(), "first run failed");
    let first_err = String::from_utf8_lossy(&first.stderr).to_string();
    assert!(
        first_err.contains("configured"),
        "first run had no work: {}",
        first_err
    );

    // Snapshot a key file's bytes — second run must leave it untouched.
    let settings_path = home.path().join(".claude/settings.json");
    let before = fs::read(&settings_path).expect("settings.json was created");

    // second install — should report 0 configured items.
    let second = Command::new(ig_bin())
        .args(["setup", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig setup ran (2)");
    assert!(second.status.success(), "second run failed");
    let after = fs::read(&settings_path).expect("settings.json still present");
    assert_eq!(before, after, "second run mutated settings.json");
}
