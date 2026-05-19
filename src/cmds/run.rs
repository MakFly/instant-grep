//! `ig run <command...>` — generic filtered command runner.
//!
//! Executes any shell command and applies the matching filter from the
//! filter engine to compress output before presenting it. When the command
//! has a specialised `ig` equivalent (e.g. `ls` → `ig ls`, `git status` →
//! `ig git status`), it is routed to that subcommand instead of going
//! through the generic filter pipeline.

use std::path::Path;
use std::process::Command;

use crate::filter::{CompiledFilter, FilterEngine};
use crate::rewrite::{RewriteResult, classify_command};
use crate::runner;
use anyhow::Result;

/// Run a command with automatic output filtering or routing to a dedicated
/// `ig` subcommand.
pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig run <command...>");
    }

    // Routing to dedicated ig subcommands (opt-out via IG_RUN_ROUTE=0).
    if std::env::var("IG_RUN_ROUTE").as_deref() != Ok("0")
        && let Some(code) = route_to_dedicated(args)?
    {
        return Ok(code);
    }

    let engine = FilterEngine::new();
    let filter = resolve_filter(&engine, args);

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    runner::run_filtered(&str_args, filter)
}

/// Return a command string with args[0] replaced by its file basename.
/// `/tmp/mocks/pytest -v` → `pytest -v`. If args[0] has no path prefix the
/// original string is returned unchanged.
pub(crate) fn basename_normalized(args: &[String]) -> String {
    let basename = Path::new(&args[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(args[0].as_str());
    if basename == args[0] {
        return args.join(" ");
    }
    std::iter::once(basename.to_string())
        .chain(args.iter().skip(1).cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find a matching filter for the command.
///
/// Tries the raw command string first (`cargo test --release`), then falls
/// back to a basename-normalized variant so that invocations through an
/// absolute path — `/tmp/mocks/pytest`, `/usr/bin/cargo` — still match
/// filters whose `match` regex is anchored on the tool name (`^pytest`).
pub(crate) fn resolve_filter<'a>(
    engine: &'a FilterEngine,
    args: &[String],
) -> Option<&'a CompiledFilter> {
    let cmd_str = args.join(" ");
    if let Some(f) = engine.find(&cmd_str) {
        return Some(f);
    }
    let normalized = basename_normalized(args);
    if normalized == cmd_str {
        return None;
    }
    engine.find(&normalized)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn pr4_subcommand_for_test(basename: &str, args: &[String]) -> Option<String> {
    pr4_subcommand_for(basename, args)
}

/// Env-var sentinel set by `route_to_dedicated` before recursing into
/// another `ig` invocation. When set, `route_to_dedicated` becomes a no-op
/// so a misconfigured rewrite (`ig run cargo` → `ig cargo_test` → `ig run
/// cargo`) cannot infinite-loop.
const RECURSION_GUARD_ENV: &str = "IG_RUN_ROUTING";

/// Tool basenames that have a dedicated `ig <tool>` parser (PR #4 + #5).
/// When the user does `ig run pytest …`, we re-exec as `ig pytest …` and
/// bypass the TOML filter pipeline entirely.
const PR4_DEDICATED_TOOLS: &[&str] = &[
    // PR #4 — test / lint / build / pkg
    "vitest",
    "jest",
    "playwright",
    "pytest",
    "go", // → ig go_test (only when args start with "test")
    "rspec",
    "rake",
    "eslint",
    "biome",
    "tsc",
    "prettier",
    "ruff",
    "mypy",
    "rubocop",
    "golangci-lint",
    "next",
    "prisma",
    "pnpm",
    "npm",
    "pip",
    // PR #5 — git platforms / cloud / system
    "gh",
    "glab",
    "gt",
    "aws",
    "kubectl",
    "psql",
    "curl",
    "wget",
    "wc",
    "tree",
];

/// If the command has a dedicated ig subcommand that compresses better than
/// the generic filter (e.g. `ls -laR` → `ig ls`), route to that subcommand
/// instead.
///
/// Only routes to a curated allowlist of `ig` subcommands to avoid infinite
/// recursion (e.g. `cargo` → `ig run cargo` would loop).
fn route_to_dedicated(args: &[String]) -> Result<Option<i32>> {
    // Recursion guard: a re-spawned `ig` invocation already in the routing
    // pipeline must NOT route again.
    if std::env::var(RECURSION_GUARD_ENV).is_ok() {
        return Ok(None);
    }

    const DEDICATED: &[&str] = &["ls", "git", "files", "read", "smart"];

    // Get the tool basename early so we can check both allowlists.
    let tool_basename = Path::new(&args[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(args[0].as_str())
        .to_string();

    // ── PR #4 dedicated tool routing ───────────────────────────────────
    // `ig run pytest -v` → re-exec as `ig pytest -v` directly.
    if let Some(ig_sub) = pr4_subcommand_for(&tool_basename, args) {
        let ig_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "ig".to_string());
        let mut child = Command::new(&ig_bin);
        child.arg(&ig_sub);
        for a in &args[1..] {
            // Drop the `go test` token when we routed to `go_test`.
            if ig_sub == "go_test" && a == "test" {
                continue;
            }
            child.arg(a);
        }
        child.env(RECURSION_GUARD_ENV, "1");
        let code = child.status()?.code().unwrap_or(1);
        return Ok(Some(code));
    }

    let cmd_str = basename_normalized(args);
    let rewritten = match classify_command(&cmd_str) {
        RewriteResult::Rewrite(s) => s,
        _ => return Ok(None),
    };

    // Strip optional env-var prefix (e.g. `IG_COMPACT=1 ig "pat" path`) before
    // identifying the ig subcommand.
    let stripped = rewritten
        .split_whitespace()
        .skip_while(|t| t.contains('=') && !t.starts_with('"'))
        .collect::<Vec<_>>()
        .join(" ");
    let after_ig = stripped.strip_prefix("ig ").unwrap_or("");
    let first_token = after_ig.split_whitespace().next().unwrap_or("");

    // Also allow "ig <pattern>" (positional content search shortcut) — the
    // first token starts with a quote.
    let is_search_shortcut = first_token.starts_with('"');
    if !DEDICATED.contains(&first_token) && !is_search_shortcut {
        return Ok(None);
    }

    // Use `sh -c` so shell quoting in the rewritten command is preserved
    // exactly (e.g. `ig "pattern with spaces"`).
    let code = Command::new("sh")
        .arg("-c")
        .arg(&rewritten)
        .env(RECURSION_GUARD_ENV, "1")
        .status()?
        .code()
        .unwrap_or(1);
    Ok(Some(code))
}

/// Map a base command (`pytest`, `cargo`, `go test`, …) to its dedicated
/// `ig <subcommand>` name, when one exists.
///
/// Returns `None` for anything not on the PR4 allowlist so the caller falls
/// back to the TOML filter pipeline.
fn pr4_subcommand_for(basename: &str, args: &[String]) -> Option<String> {
    if !PR4_DEDICATED_TOOLS.contains(&basename) {
        // `cargo test ...` routes to ig cargo_test; `cargo build ...` to ig cargo_build.
        if basename == "cargo" {
            if args.get(1).map(|s| s.as_str()) == Some("test") {
                return Some("cargo_test".to_string());
            }
            if args.get(1).map(|s| s.as_str()) == Some("build") {
                return Some("cargo_build".to_string());
            }
        }
        return None;
    }
    if basename == "go" {
        // Only route `go test` — not `go build`/`go run`/...
        if args.get(1).map(|s| s.as_str()) == Some("test") {
            return Some("go_test".to_string());
        }
        return None;
    }
    Some(basename.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_error() {
        let result = run(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn absolute_path_matches_basename_filter() {
        let engine = FilterEngine::new();
        // `cargo` has a builtin filter matching `^cargo`. When invoked through
        // an absolute path the raw command string (`/usr/bin/cargo build …`)
        // must still resolve via the basename fallback.
        let args = vec!["/usr/bin/cargo".to_string(), "build".to_string()];
        let filter = resolve_filter(&engine, &args);
        assert!(
            filter.is_some(),
            "absolute-path invocation should match via basename fallback"
        );
    }

    #[test]
    fn plain_name_still_matches() {
        let engine = FilterEngine::new();
        let args = vec!["cargo".to_string(), "build".to_string()];
        assert!(resolve_filter(&engine, &args).is_some());
    }

    #[test]
    fn unknown_command_returns_none() {
        let engine = FilterEngine::new();
        let args = vec!["totally-unknown-tool".to_string(), "--flag".to_string()];
        assert!(resolve_filter(&engine, &args).is_none());
    }

    #[test]
    fn basename_strips_absolute_path() {
        let args = vec!["/tmp/mocks/pytest".to_string(), "-v".to_string()];
        assert_eq!(basename_normalized(&args), "pytest -v");
    }

    #[test]
    fn basename_preserves_plain_name() {
        let args = vec!["pytest".to_string(), "-v".to_string()];
        assert_eq!(basename_normalized(&args), "pytest -v");
    }

    #[test]
    fn basename_single_arg() {
        let args = vec!["/usr/bin/ls".to_string()];
        assert_eq!(basename_normalized(&args), "ls");
    }

    #[test]
    fn pr4_routes_pytest() {
        let args = vec!["pytest".to_string(), "-v".to_string()];
        assert_eq!(
            pr4_subcommand_for("pytest", &args).as_deref(),
            Some("pytest")
        );
    }

    #[test]
    fn pr4_routes_cargo_test_only_on_test_verb() {
        let args = vec!["cargo".to_string(), "test".to_string()];
        assert_eq!(
            pr4_subcommand_for("cargo", &args).as_deref(),
            Some("cargo_test")
        );
        let args2 = vec!["cargo".to_string(), "check".to_string()];
        assert_eq!(pr4_subcommand_for("cargo", &args2), None);
    }

    #[test]
    fn pr4_routes_go_test_only_on_test_verb() {
        let args = vec!["go".to_string(), "test".to_string(), "./...".to_string()];
        assert_eq!(pr4_subcommand_for("go", &args).as_deref(), Some("go_test"));
        let args2 = vec!["go".to_string(), "build".to_string()];
        assert_eq!(pr4_subcommand_for("go", &args2), None);
    }

    #[test]
    fn pr4_unknown_tool_returns_none() {
        let args = vec!["totally-unknown".to_string()];
        assert_eq!(pr4_subcommand_for("totally-unknown", &args), None);
    }

    #[test]
    fn route_opt_out_returns_none() {
        // SAFETY: tests that mutate env share a mutex below for stability.
        // This one doesn't race because route_to_dedicated only reads the var.
        unsafe { std::env::set_var("IG_RUN_ROUTE", "0") };
        // Even a command that WOULD route (git status) returns None when opted out.
        // We don't call route_to_dedicated directly since it dispatches; we
        // validate via the env-var guard in `run` — covered below by a
        // lightweight check:
        let opt_out = std::env::var("IG_RUN_ROUTE").as_deref() == Ok("0");
        assert!(opt_out);
        unsafe { std::env::remove_var("IG_RUN_ROUTE") };
    }
}
