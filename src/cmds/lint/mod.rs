//! Per-tool linter / formatter modules. Each `pub fn run(args, opts)`
//! spawns the underlying binary, asks for JSON when supported, and prints
//! a compact `LintResult` via `TokenFormatter`.

#![allow(dead_code)]

pub mod biome;
pub mod eslint;
pub mod golangci;
pub mod mypy;
pub mod prettier;
pub mod rubocop;
pub mod ruff;
pub mod tsc;

use crate::RunOptions;
use anyhow::Result;

pub fn dispatch(tool: &str, args: &[String], opts: RunOptions) -> Result<i32> {
    match tool {
        "eslint" => eslint::run(args, opts),
        "biome" => biome::run(args, opts),
        "tsc" => tsc::run(args, opts),
        "prettier" => prettier::run(args, opts),
        "ruff" => ruff::run(args, opts),
        "mypy" => mypy::run(args, opts),
        "rubocop" => rubocop::run(args, opts),
        "golangci" | "golangci-lint" | "golangci_lint" => golangci::run(args, opts),
        other => anyhow::bail!("unknown lint tool: {}", other),
    }
}
