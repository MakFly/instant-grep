//! PR #5 — markdown body filter parity tests.
//!
//! Verifies `ig __parse markdown` strips HTML comments, badges, images,
//! horizontal rules, collapses blank runs, and preserves code fences.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_markdown(body: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_ig");
    let mut child = Command::new(bin)
        .args(["__parse", "markdown"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ig");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "__parse markdown failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

#[test]
fn strips_html_comments_and_badges() {
    let body = "title\n\n[![CI](https://shields/io)](https://x)\n\nsummary <!-- skip --> here\n";
    let out = run_markdown(body);
    assert!(out.contains("title"));
    assert!(out.contains("summary"));
    assert!(out.contains("here"));
    assert!(!out.contains("shields/io"));
    assert!(!out.contains("skip"));
}

#[test]
fn preserves_fenced_code_blocks() {
    let body = "before\n\n```\n<!-- not stripped -->\n```\n\nafter";
    let out = run_markdown(body);
    assert!(out.contains("<!-- not stripped -->"));
}

#[test]
fn collapses_multiple_blank_lines() {
    let body = "a\n\n\n\n\nb";
    let out = run_markdown(body);
    assert_eq!(out, "a\n\nb");
}
