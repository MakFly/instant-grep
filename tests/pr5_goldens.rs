//! PR #5 — golden-file runner. Pipes each fixture's `raw.txt` through
//! `ig __parse <parser-name>` and compares against the golden expected
//! output.

use pretty_assertions::assert_eq;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pr5")
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
        .expect("spawn ig");
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

fn assert_simple(dir: &str, parser: &str) {
    let dir = fixtures_root().join(dir);
    let raw = std::fs::read_to_string(dir.join("raw.txt")).expect("raw.txt");
    let expected = std::fs::read_to_string(dir.join("expected.txt")).expect("expected.txt");
    let got = run_parser(parser, &raw, false);
    assert_eq!(got, expected, "{}", parser);
}

fn assert_compact_ultra(dir: &str, parser: &str) {
    let dir = fixtures_root().join(dir);
    let raw = std::fs::read_to_string(dir.join("raw.txt")).expect("raw.txt");
    let expected_compact =
        std::fs::read_to_string(dir.join("expected.compact.txt")).expect("expected.compact.txt");
    let expected_ultra =
        std::fs::read_to_string(dir.join("expected.ultra.txt")).expect("expected.ultra.txt");
    let got_compact = run_parser(parser, &raw, false);
    let got_ultra = run_parser(parser, &raw, true);
    assert_eq!(got_compact, expected_compact, "{} compact", parser);
    assert_eq!(got_ultra, expected_ultra, "{} ultra", parser);
}

#[test]
fn gh_pr_list() {
    assert_compact_ultra("gh_pr_list", "gh-pr-list");
}
#[test]
fn aws_sts() {
    assert_simple("aws_sts", "aws-sts");
}
#[test]
fn aws_ec2() {
    assert_simple("aws_ec2", "aws-ec2");
}
#[test]
fn kubectl_get() {
    assert_simple("kubectl_get", "kubectl-get");
}
#[test]
fn glab_mr_list() {
    assert_simple("glab_mr_list", "glab-mr-list");
}
