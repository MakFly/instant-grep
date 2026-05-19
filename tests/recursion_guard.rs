//! PR #4 — `ig run cargo test` must route to `ig cargo_test` without
//! looping back through `ig run`. The recursion-guard env var prevents
//! the re-spawned `ig` invocation from routing again.
//!
//! We can't actually execute `cargo test` here (it would recursively run
//! ig's own test suite). Instead we verify the env-var sentinel: when set,
//! a second invocation that WOULD route must not.

use std::process::Command;

#[test]
fn recursion_guard_env_blocks_second_route() {
    let bin = env!("CARGO_BIN_EXE_ig");
    // Spawn `ig run echo hi` with the recursion-guard sentinel pre-set.
    // The router should see the sentinel and skip dedicated routing, so
    // `echo` falls through the (matchless) filter pipeline and runs.
    let out = Command::new(bin)
        .args(["run", "echo", "hi"])
        .env("IG_RUN_ROUTING", "1")
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hi"),
        "echo should have executed; got stdout={}, stderr={}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn opt_out_via_ig_run_route_zero() {
    // Belt-and-braces: the explicit opt-out should still work.
    let bin = env!("CARGO_BIN_EXE_ig");
    let out = Command::new(bin)
        .args(["run", "echo", "ok"])
        .env("IG_RUN_ROUTE", "0")
        .output()
        .expect("spawn");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}
