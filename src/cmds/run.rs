//! `ig run <command...>` — generic filtered command runner.
//!
//! Executes any shell command and applies the matching filter from the
//! filter engine to compress output before presenting it.

use crate::filter::FilterEngine;
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
    let cmd_str = args.join(" ");
    let filter = engine.find(&cmd_str);

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    runner::run_filtered(&str_args, filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_error() {
        let result = run(&[]);
        assert!(result.is_err());
    }
}
