//! PR #4 — verify pytest XFAIL / XPASS surfacing.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_ig");
    let mut child = Command::new(bin)
        .args(["__parse", "pytest"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn xfailed_counted_as_skipped() {
    let s = "===== 5 passed, 2 xfailed in 0.10s =====\n";
    let out = run(s);
    assert!(out.contains("5 passed"));
    assert!(
        out.contains("2 skipped"),
        "xfailed should bucket into skipped; out={}",
        out
    );
    assert!(out.contains("0 failed"));
}

#[test]
fn xpassed_surfaced_as_failure() {
    let s = "===== 2 passed, 1 xpassed in 0.10s =====\n";
    let out = run(s);
    assert!(
        out.contains("1 failed"),
        "xpassed must inflate failed count; out={}",
        out
    );
    assert!(
        out.contains("xpass") || out.contains("xfail"),
        "failure description should mention xpass; out={}",
        out
    );
}
