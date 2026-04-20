//! `ig run <command...>` — generic filtered command runner.
//!
//! Executes any shell command and applies the matching filter from the
//! filter engine to compress output before presenting it.

use std::path::Path;

use crate::filter::{CompiledFilter, FilterEngine};
use crate::runner;
use anyhow::Result;

/// Run a command with automatic output filtering.
///
/// Looks up the command string in the filter engine and, if a match is found,
/// pipes the output through the filter pipeline before printing.
pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig run <command...>");
    }

    let engine = FilterEngine::new();
    let filter = resolve_filter(&engine, args);

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    runner::run_filtered(&str_args, filter)
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
    let basename = Path::new(&args[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(args[0].as_str());
    if basename == args[0] {
        return None;
    }
    let basename_cmd = std::iter::once(basename.to_string())
        .chain(args.iter().skip(1).cloned())
        .collect::<Vec<_>>()
        .join(" ");
    engine.find(&basename_cmd)
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
}
