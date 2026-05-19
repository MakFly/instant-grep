//! PR #4 — unified golden runner. For each fixture directory in
//! `tests/fixtures/<tool>/`, feed `raw.txt` through `ig __parse <tool>`
//! and assert the output matches `expected.compact.txt` /
//! `expected.ultra.txt`.

use pretty_assertions::assert_eq;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run_parser(tool: &str, raw: &str, ultra: bool) -> String {
    let bin = env!("CARGO_BIN_EXE_ig");
    let mut cmd = Command::new(bin);
    if ultra {
        cmd.arg("--ultra-compact");
    }
    cmd.args(["__parse", tool]);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(raw.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "ig __parse {} failed: {}",
        tool,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

fn assert_golden(tool: &str) {
    let dir = fixtures_root().join(tool);
    let raw = std::fs::read_to_string(dir.join("raw.txt"))
        .unwrap_or_else(|e| panic!("read {}/raw.txt: {}", tool, e));
    let expected_compact = std::fs::read_to_string(dir.join("expected.compact.txt"))
        .unwrap_or_else(|e| panic!("read {}/expected.compact.txt: {}", tool, e));
    let expected_ultra = std::fs::read_to_string(dir.join("expected.ultra.txt"))
        .unwrap_or_else(|e| panic!("read {}/expected.ultra.txt: {}", tool, e));

    let got_compact = run_parser(tool, &raw, false);
    assert_eq!(got_compact, expected_compact, "{} compact mismatch", tool);

    let got_ultra = run_parser(tool, &raw, true);
    assert_eq!(got_ultra, expected_ultra, "{} ultra mismatch", tool);
}

#[test]
fn vitest_golden() {
    assert_golden("vitest");
}
#[test]
fn jest_golden() {
    assert_golden("jest");
}
#[test]
fn pytest_golden() {
    assert_golden("pytest");
}
#[test]
fn playwright_golden() {
    assert_golden("playwright");
}
#[test]
fn go_test_golden() {
    assert_golden("go_test");
}
#[test]
fn rspec_golden() {
    assert_golden("rspec");
}
#[test]
fn golangci_golden() {
    assert_golden("golangci");
}
