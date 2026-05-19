#![allow(dead_code)]
//! Output parser trait + shared types for tool-output compression.
//!
//! Foundation for PR #4 (test-runner parsers). Concrete parsers (vitest,
//! jest, pytest, cargo test, …) implement `OutputParser` to convert raw
//! stdout/stderr into a typed `TestResult` (or other typed output), which
//! the `TokenFormatter` then renders in compact / ultra-compact form.

pub mod formatter;
pub mod types;

#[allow(unused_imports)]
pub use formatter::TokenFormatter;
#[allow(unused_imports)]
pub use types::{FormatMode, TestFailure, TestResult};

/// Result of parsing a tool's raw output.
///
/// - `Full(value)`: parsing succeeded — every section was understood.
/// - `Partial { value, warning }`: parsing produced a useful value, but
///   some portion was unrecognised. The warning explains what was skipped.
/// - `Failed { reason, passthrough }`: parsing could not produce anything
///   meaningful. The caller should fall back to printing `passthrough`
///   (typically the raw input) so the user still sees something.
pub enum ParseResult<T> {
    Full(T),
    Partial { value: T, warning: String },
    Failed { reason: String, passthrough: String },
}

impl<T> ParseResult<T> {
    /// True iff parsing produced a usable value (Full or Partial).
    pub fn is_ok(&self) -> bool {
        matches!(self, ParseResult::Full(_) | ParseResult::Partial { .. })
    }

    /// Extract the value if available (Full or Partial).
    pub fn value(self) -> Option<T> {
        match self {
            ParseResult::Full(v) => Some(v),
            ParseResult::Partial { value, .. } => Some(value),
            ParseResult::Failed { .. } => None,
        }
    }
}

/// Trait for tool-output parsers.
///
/// Each implementor converts a raw `&str` (stdout/stderr) into a typed
/// `Output`. The associated type lets one parser produce a `TestResult`,
/// another a `LintResult`, etc.
pub trait OutputParser {
    type Output;
    fn parse(input: &str) -> ParseResult<Self::Output>;
}
