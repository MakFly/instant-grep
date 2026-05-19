//! `ig setup --agent claude --hook-only` must not touch the rules files.

use std::fs;
use std::process::Command;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn hook_only_skips_claude_md_and_rules_file() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();

    let out = Command::new(ig_bin())
        .args(["setup", "--agent", "claude", "--hook-only"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig setup ran");
    assert!(out.status.success());

    // CLAUDE.md and rules/tools/ig.md must NOT have been created.
    assert!(
        !home.path().join(".claude/CLAUDE.md").exists(),
        "--hook-only must not write CLAUDE.md"
    );
    assert!(
        !home.path().join(".claude/rules/tools/ig.md").exists(),
        "--hook-only must not write rules/tools/ig.md"
    );

    // But the hook scripts SHOULD be installed.
    assert!(
        home.path().join(".claude/hooks/ig-guard.sh").exists(),
        "--hook-only must install hook scripts"
    );
    assert!(
        home.path().join(".claude/settings.json").exists(),
        "--hook-only must patch settings.json"
    );
}
