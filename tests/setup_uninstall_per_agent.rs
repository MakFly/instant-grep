//! `--uninstall --agent claude` must remove claude artifacts only, leaving
//! the codex AGENTS.md managed-block intact.

use std::fs;
use std::process::Command;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn uninstall_claude_only_leaves_codex_untouched() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::create_dir_all(home.path().join(".codex")).unwrap();

    // Install both.
    Command::new(ig_bin())
        .args(["setup", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .unwrap();
    Command::new(ig_bin())
        .args(["setup", "--agent", "codex"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .unwrap();

    let codex_md = home.path().join(".codex/AGENTS.md");
    let claude_md = home.path().join(".claude/CLAUDE.md");
    assert!(claude_md.exists(), "claude install failed");
    assert!(codex_md.exists(), "codex install failed");
    assert!(
        fs::read_to_string(&codex_md)
            .unwrap()
            .contains("IG-MANAGED-BLOCK:BEGIN")
    );

    // Uninstall claude only.
    let out = Command::new(ig_bin())
        .args(["setup", "--uninstall", "--agent", "claude"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("uninstall ran");
    assert!(out.status.success());

    // claude artifacts gone, codex untouched.
    assert!(
        !home.path().join(".claude/hooks/ig-guard.sh").exists(),
        "claude hook should be removed"
    );
    assert!(
        !home.path().join(".claude/rules/tools/ig.md").exists(),
        "claude rules file should be removed"
    );
    let codex_after = fs::read_to_string(&codex_md).unwrap();
    assert!(
        codex_after.contains("IG-MANAGED-BLOCK:BEGIN"),
        "codex managed-block must survive claude uninstall"
    );
}
