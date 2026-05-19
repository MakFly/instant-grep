//! `ig pytest [args...]` — wrap pytest with `--tb=short -q` and parse its
//! text summary. Tracks passed/failed/skipped/xfail/xpass.

use anyhow::Result;
use regex::Regex;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
    strip_ansi,
};
use crate::parser::{ParseResult, TestFailure, TestResult, TokenFormatter};

/// Parse pytest's terminal summary line, e.g.
/// `===== 3 failed, 5 passed, 1 skipped, 2 xfailed, 1 xpassed in 0.42s =====`.
///
/// XPASSED (a test marked `xfail` that unexpectedly passed) is counted as a
/// FAIL — the same way pytest itself flags it red by default.
pub fn parse(input: &str) -> ParseResult<TestResult> {
    let cleaned = strip_ansi(input);
    let summary_line = cleaned
        .lines()
        .rev()
        .find(|l| l.contains(" passed") || l.contains(" failed") || l.contains(" error"));
    let Some(line) = summary_line else {
        return ParseResult::Failed {
            reason: "pytest: no summary line".to_string(),
            passthrough: input.to_string(),
        };
    };

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut xfailed = 0u32;
    let mut xpassed = 0u32;
    let mut errors = 0u32;
    let count_re = Regex::new(r"(\d+)\s+(passed|failed|skipped|xfailed|xpassed|errors?)").unwrap();
    for caps in count_re.captures_iter(line) {
        let n: u32 = caps[1].parse().unwrap_or(0);
        match &caps[2] {
            "passed" => passed = n,
            "failed" => failed = n,
            "skipped" => skipped = n,
            "xfailed" => xfailed = n,
            "xpassed" => xpassed = n,
            "error" | "errors" => errors = n,
            _ => {}
        }
    }
    let dur_re = Regex::new(r"in\s+([\d.]+)s").unwrap();
    let duration_ms = dur_re
        .captures(line)
        .and_then(|c| c[1].parse::<f64>().ok())
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(0);

    let mut failures = extract_failures(&cleaned);
    if xpassed > 0 {
        failures.push(TestFailure {
            name: format!("<{} unexpected xpass>", xpassed),
            file: None,
            line: None,
            message: "test marked xfail but passed".to_string(),
            snippet: None,
        });
    }

    let total_fail = failed + xpassed + errors;
    if passed + failed + skipped + xfailed + xpassed + errors == 0 {
        return ParseResult::Failed {
            reason: "pytest: no counts in summary".to_string(),
            passthrough: input.to_string(),
        };
    }

    let mut r = TestResult {
        passed,
        failed: total_fail,
        skipped: skipped + xfailed,
        duration_ms,
        failures,
    };
    // Keep failures from outgrowing the summary in extreme cases.
    if r.failures.len() > 200 {
        r.failures.truncate(200);
    }
    ParseResult::Full(r)
}

/// Pull FAILED lines from the pytest short summary block.
fn extract_failures(cleaned: &str) -> Vec<TestFailure> {
    let mut out = Vec::new();
    // Pytest short-summary block ends with the dashed summary line. Look for
    // lines beginning with "FAILED " in the cleaned output.
    let re =
        Regex::new(r"^FAILED\s+([^\s:]+)(?::(\d+))?(?:::([^\s-]+))?\s*(?:-\s*(.*))?$").unwrap();
    for line in cleaned.lines() {
        let line = line.trim_end();
        if let Some(c) = re.captures(line) {
            let file = c.get(1).map(|m| m.as_str().to_string());
            let line_n = c.get(2).and_then(|m| m.as_str().parse().ok());
            let name = c
                .get(3)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| file.clone().unwrap_or_default());
            let msg = c.get(4).map(|m| m.as_str().to_string()).unwrap_or_default();
            out.push(TestFailure {
                name,
                file,
                line: line_n,
                message: msg,
                snippet: None,
            });
        }
    }
    out
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["pytest".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("pytest") {
        user.remove(0);
    }
    let has_tb = user.iter().any(|a| a.starts_with("--tb"));
    let has_q = user.iter().any(|a| a == "-q" || a == "--quiet");
    argv.extend(user);
    if !has_tb {
        argv.push("--tb=short".to_string());
    }
    if !has_q {
        argv.push("-q".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig pytest {}", args.join(" "));
    let (output, outcome) = match parse(&run.merged) {
        ParseResult::Full(r) => (
            TokenFormatter::new().format_test_result(&r, opts.format_mode),
            ParseOutcome::Full,
        ),
        ParseResult::Partial { value, warning } => {
            let mut s = TokenFormatter::new().format_test_result(&value, opts.format_mode);
            s.push_str(&format!("\n(partial: {})\n", warning));
            (s, ParseOutcome::Partial)
        }
        ParseResult::Failed { passthrough, .. } => match try_toml_filter(&argv, &run.merged) {
            Some(f) => (f, ParseOutcome::Passthrough),
            None => (
                format!("{}\n(parse fallback: passthrough)\n", passthrough),
                ParseOutcome::Passthrough,
            ),
        },
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_summary() {
        let s = "============ 3 passed, 1 failed in 0.42s ============\n";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 3);
                assert_eq!(r.failed, 1);
                assert_eq!(r.duration_ms, 420);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn surfaces_xpass_as_failure() {
        let s = "===== 2 passed, 1 xpassed in 0.10s =====\n";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 2);
                assert_eq!(r.failed, 1); // xpassed counted as fail
                assert!(r.failures.iter().any(|f| f.name.contains("xpass")));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn xfail_counted_as_skipped() {
        let s = "===== 5 passed, 2 xfailed in 0.10s =====\n";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 5);
                assert_eq!(r.failed, 0);
                assert_eq!(r.skipped, 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn extracts_failed_lines() {
        let s = "\
FAILED tests/test_foo.py::test_bar - AssertionError: x != y
FAILED tests/test_baz.py
===== 1 passed, 2 failed in 1.5s =====
";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.failed, 2);
                assert_eq!(r.failures.len(), 2);
                assert_eq!(r.failures[0].file.as_deref(), Some("tests/test_foo.py"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn failed_parse_returns_passthrough() {
        match parse("garbage output") {
            ParseResult::Failed { .. } => {}
            _ => panic!(),
        }
    }
}
