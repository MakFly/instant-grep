//! Per-tool package-manager wrappers — surface install/add/remove summaries
//! while suppressing spammy progress lines.

#![allow(dead_code)]

pub mod npm;
pub mod pip;
pub mod pnpm;

use crate::RunOptions;
use anyhow::Result;

pub fn dispatch(tool: &str, args: &[String], opts: RunOptions) -> Result<i32> {
    match tool {
        "pnpm" => pnpm::run(args, opts),
        "npm" => npm::run(args, opts),
        "pip" => pip::run(args, opts),
        other => anyhow::bail!("unknown pkg tool: {}", other),
    }
}

/// Drop spammy progress lines that node package managers emit by the
/// thousand (`progress`, `█` bars, `idealTree`, `npm http fetch`, …).
pub fn drop_progress(raw: &str) -> String {
    let drop_prefixes = [
        "npm http",
        "npm timing",
        "npm verb",
        "npm sill",
        "npm info ",
        "npm WARN deprecated",
    ];
    let mut keep = Vec::new();
    for line in raw.lines() {
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        if drop_prefixes.iter().any(|p| t.starts_with(p)) {
            continue;
        }
        if t.contains('█') || t.contains("idealTree") || t.contains("reify") {
            continue;
        }
        keep.push(line);
    }
    keep.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drops_npm_http_lines() {
        let s = "npm http fetch GET 200 https://x\nadded 5 packages\n";
        let out = drop_progress(s);
        assert!(out.contains("added 5"));
        assert!(!out.contains("npm http"));
    }
}
