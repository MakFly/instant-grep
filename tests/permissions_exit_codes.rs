//! Exit-code mapping for `ig rewrite` under the RTK 0/1/2/3 protocol.
//!
//!   0 → passthrough (familiar command, nothing to do)
//!   1 → rewrite suggestion / ask — surface to user
//!   2 → deny — block
//!   3 → Default + unfamiliar — prompt the user (load-bearing #1155 sentinel)
//!
//! The protocol is implemented in `src/rewrite.rs::run_rewrite`. We exercise
//! the built binary so the test mirrors what shell hooks observe in
//! production.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

fn run(cmd: &str) -> i32 {
    let out = Command::new(bin())
        .args(["rewrite", cmd])
        .env_remove("IG_HOOK_EXIT_LEGACY")
        .output()
        .expect("spawn ig rewrite");
    out.status.code().expect("ig rewrite exited via signal")
}

#[test]
fn allow_familiar_no_rewrite_is_zero() {
    // `ls` is in KNOWN_GOOD_PREFIXES, no rewrite, no permission rule → 0.
    assert_eq!(run("ls"), 0);
    assert_eq!(run("pwd"), 0);
    assert_eq!(run("echo hi"), 0);
}

#[test]
fn rewrite_suggestion_is_one() {
    // `head src/main.rs` rewrites to `ig read src/main.rs` → 1.
    assert_eq!(run("head src/main.rs"), 1);
    // `cargo test` routes through `ig run cargo test` → 1.
    assert_eq!(run("cargo test"), 1);
}

#[test]
fn deny_is_two() {
    assert_eq!(run("rm -rf /"), 2);
    assert_eq!(run("git reset --hard HEAD~1"), 2);
    assert_eq!(run("mkfs.ext4 /dev/sda1"), 2);
}

#[test]
fn ask_is_one() {
    // Built-in ask rule (git push --force) → exit 1.
    assert_eq!(run("git push --force origin main"), 1);
    // npm publish → ask → 1.
    assert_eq!(run("npm publish"), 1);
}

#[test]
fn unfamiliar_default_is_three() {
    // No permission rule, no known-good prefix, no rewrite → 3 (rtk #1155).
    assert_eq!(run("xenophobicsoftware --weird-flag thing"), 3);
}

#[test]
fn legacy_env_collapses_to_zero() {
    let out = Command::new(bin())
        .args(["rewrite", "rm -rf /"])
        .env("IG_HOOK_EXIT_LEGACY", "1")
        .output()
        .expect("spawn ig rewrite");
    assert_eq!(out.status.code().unwrap(), 0);
}
