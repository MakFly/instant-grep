//! PR #1 — exercise the `OutputParser` trait, `ParseResult` variants,
//! and `TokenFormatter` rendering in both Compact and Ultra modes.
//!
//! `instant-grep` is a binary crate (no lib target), so the parser
//! modules are pulled in via `#[path]`.

#[path = "../src/parser/types.rs"]
mod types;

#[path = "../src/parser/formatter.rs"]
mod formatter;

// Mini local copy of `ParseResult` + `OutputParser` (since `parser/mod.rs`
// itself can't be imported as a `#[path]` without dragging in its sub-mods
// with `super::` paths).
enum ParseResult<T> {
    Full(T),
    Partial {
        value: T,
        #[allow(dead_code)]
        warning: String,
    },
    Failed {
        reason: String,
        passthrough: String,
    },
}

impl<T> ParseResult<T> {
    fn is_ok(&self) -> bool {
        matches!(self, ParseResult::Full(_) | ParseResult::Partial { .. })
    }
    fn value(self) -> Option<T> {
        match self {
            ParseResult::Full(v) | ParseResult::Partial { value: v, .. } => Some(v),
            ParseResult::Failed { .. } => None,
        }
    }
}

trait OutputParser {
    type Output;
    fn parse(input: &str) -> ParseResult<Self::Output>;
}

use formatter::TokenFormatter;
use types::{FormatMode, TestFailure, TestResult};

struct ToyParser;
impl OutputParser for ToyParser {
    type Output = TestResult;
    fn parse(input: &str) -> ParseResult<Self::Output> {
        if input.starts_with("FAIL_HARD") {
            return ParseResult::Failed {
                reason: "unrecognised header".into(),
                passthrough: input.to_string(),
            };
        }
        if input.is_empty() {
            return ParseResult::Partial {
                value: TestResult::default(),
                warning: "empty input".into(),
            };
        }
        ParseResult::Full(TestResult {
            passed: 3,
            failed: 1,
            skipped: 0,
            duration_ms: 12,
            failures: vec![TestFailure {
                name: "demo".into(),
                file: Some("a.rs".into()),
                line: Some(7),
                message: "boom".into(),
                snippet: Some("assert!(false);".into()),
            }],
        })
    }
}

#[test]
fn parse_result_full() {
    let r = ToyParser::parse("ok");
    assert!(r.is_ok());
    assert_eq!(r.value().unwrap().failed, 1);
}

#[test]
fn parse_result_partial() {
    let r = ToyParser::parse("");
    assert!(r.is_ok());
    assert_eq!(r.value().unwrap().failed, 0);
}

#[test]
fn parse_result_failed() {
    let r = ToyParser::parse("FAIL_HARD nope");
    assert!(!r.is_ok());
    match r {
        ParseResult::Failed {
            reason,
            passthrough,
        } => {
            assert!(reason.contains("unrecognised"));
            assert!(passthrough.contains("FAIL_HARD"));
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn formatter_compact_includes_snippet() {
    let r = ToyParser::parse("ok").value().unwrap();
    let out = TokenFormatter::new().format_test_result(&r, FormatMode::Compact);
    assert!(out.contains("passed"));
    assert!(out.contains("assert!(false);"));
}

#[test]
fn formatter_ultra_drops_snippet() {
    let r = ToyParser::parse("ok").value().unwrap();
    let out = TokenFormatter::new().format_test_result(&r, FormatMode::Ultra);
    assert!(!out.contains("assert!(false);"));
    assert!(out.contains("FAIL"));
}

#[test]
fn formatter_ultra_shorter_than_compact() {
    let r = ToyParser::parse("ok").value().unwrap();
    let f = TokenFormatter::new();
    let c = f.format_test_result(&r, FormatMode::Compact);
    let u = f.format_test_result(&r, FormatMode::Ultra);
    assert!(u.len() < c.len(), "ultra={} compact={}", u.len(), c.len());
}
