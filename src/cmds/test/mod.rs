//! Per-tool test runner modules. Each `pub fn run(args, opts)` spawns the
//! underlying binary with a structured reporter (JSON/NDJSON when possible)
//! and prints a compact summary built from `TestResult` + `TokenFormatter`.
//!
//! Introduced in PR #4 of the RTK-iso plan.

#![allow(dead_code)]

pub mod cargo_test;
pub mod go_test;
pub mod jest;
pub mod playwright;
pub mod pytest;
pub mod rake;
pub mod rspec;
pub mod vitest;

use crate::RunOptions;
use anyhow::Result;

/// Route by tool name. Used by `cmds::run::route_to_dedicated` and the
/// per-subcommand handlers in `main.rs`.
pub fn dispatch(tool: &str, args: &[String], opts: RunOptions) -> Result<i32> {
    match tool {
        "vitest" => vitest::run(args, opts),
        "jest" => jest::run(args, opts),
        "playwright" => playwright::run(args, opts),
        "pytest" => pytest::run(args, opts),
        "cargo_test" | "cargo-test" => cargo_test::run(args, opts),
        "go_test" | "go-test" => go_test::run(args, opts),
        "rspec" => rspec::run(args, opts),
        "rake" => rake::run(args, opts),
        other => anyhow::bail!("unknown test tool: {}", other),
    }
}
