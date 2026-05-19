//! `ig setup --agent claude --dry-run` must not touch disk.
//!
//! Drops a sentinel `CLAUDE.md` into an empty $HOME, runs the command,
//! verifies (a) exit 0, (b) the sentinel's mtime is unchanged, (c) no new
//! files were created.

use std::fs;
use std::process::Command;
use std::time::SystemTime;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn dry_run_claude_does_not_touch_disk() {
    let home = tempfile::tempdir().unwrap();
    let claude_dir = home.path().join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let md = claude_dir.join("CLAUDE.md");
    fs::write(&md, "# CLAUDE.md\nhello\n").unwrap();
    let original_mtime: SystemTime = fs::metadata(&md).unwrap().modified().unwrap();
    let original_content = fs::read_to_string(&md).unwrap();

    // sleep just enough that any write would bump mtime past our snapshot
    std::thread::sleep(std::time::Duration::from_millis(20));

    let output = Command::new(ig_bin())
        .args(["setup", "--agent", "claude", "--dry-run"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig setup ran");

    assert!(
        output.status.success(),
        "dry-run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    // CLAUDE.md must be byte-identical and have the same mtime.
    assert_eq!(fs::read_to_string(&md).unwrap(), original_content);
    let new_mtime = fs::metadata(&md).unwrap().modified().unwrap();
    assert_eq!(
        new_mtime, original_mtime,
        "dry-run modified CLAUDE.md mtime"
    );

    // The hook scripts must not have been written.
    assert!(!claude_dir.join("hooks/ig-guard.sh").exists());
    assert!(!claude_dir.join("rules/tools/ig.md").exists());
    assert!(!claude_dir.join("settings.json").exists());
}
