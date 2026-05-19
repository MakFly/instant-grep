//! Per-tool build modules — thin wrappers that spawn the underlying
//! tool, run its merged output through the TOML filter pipeline, and
//! emit via the shared `finish::emit` helper.

#![allow(dead_code)]

pub mod cargo_build;
pub mod next;
pub mod prisma;

use crate::RunOptions;
use anyhow::Result;

pub fn dispatch(tool: &str, args: &[String], opts: RunOptions) -> Result<i32> {
    match tool {
        "next" => next::run(args, opts),
        "prisma" => prisma::run(args, opts),
        "cargo_build" | "cargo-build" => cargo_build::run(args, opts),
        other => anyhow::bail!("unknown build tool: {}", other),
    }
}
