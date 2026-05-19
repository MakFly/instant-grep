//! `ig setup --agent foobar` must exit non-zero with a readable error.

use std::process::Command;

fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

#[test]
fn unknown_agent_exits_nonzero_with_message() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(ig_bin())
        .args(["setup", "--agent", "foobar"])
        .env("HOME", home.path())
        .env_remove("SUDO_USER")
        .output()
        .expect("ig ran");
    assert!(!out.status.success(), "unknown agent should fail");
    let s = String::from_utf8_lossy(&out.stderr);
    assert!(
        s.contains("foobar"),
        "error must mention the bad agent: {}",
        s
    );
    assert!(s.contains("valid:"), "error must list valid agents: {}", s);
}
