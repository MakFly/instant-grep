//! Shared utilities for the per-tool Rust parsers in `src/cmds/{test,lint,build,pkg}`.
//!
//! Introduced in PR #4 of the RTK-iso plan.

#![allow(dead_code)]

pub mod ansi;
pub mod extract_json;
pub mod finish;
pub mod markdown;
pub mod package_manager;
pub mod spawn;
pub mod truncate;

pub use ansi::strip_ansi;
pub use extract_json::{extract_json_lines, extract_json_object};

/// Common return type used by the per-tool dispatchers in `cmds::test::dispatch`,
/// `cmds::lint::dispatch`, etc.
///
/// Mirrors `parser::ParseResult` outcomes for the SQLite `parse_outcome` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    /// Full structured parse (JSON, NDJSON, well-formed text) succeeded.
    Full,
    /// Parsing succeeded but some part of the input was unrecognised.
    Partial,
    /// Could not parse — fell through to TOML filter or raw passthrough.
    Passthrough,
    /// Spawning the wrapped binary or another I/O step failed.
    Error,
}

impl ParseOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            ParseOutcome::Full => "full",
            ParseOutcome::Partial => "partial",
            ParseOutcome::Passthrough => "passthrough",
            ParseOutcome::Error => "error",
        }
    }
}
