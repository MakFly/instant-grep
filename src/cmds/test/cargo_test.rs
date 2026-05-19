//! `ig cargo_test [args...]` — wraps `cargo test --no-fail-fast` and
//! parses the standard `test result:` summary lines (one per crate).

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

/// Parse `cargo test` output. We aggregate counts across every
/// `test result: …` line because a workspace can emit several.
pub fn parse(input: &str) -> ParseResult<TestResult> {
    let cleaned = strip_ansi(input);
    let summary_re = Regex::new(
        r"test result:\s+(?:ok|FAILED)\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored(?:;\s+(\d+)\s+measured)?(?:;\s+(\d+)\s+filtered out)?(?:;\s+finished in\s+([\d.]+)s)?",
    )
    .unwrap();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut duration_ms = 0u64;
    let mut seen = false;
    for caps in summary_re.captures_iter(&cleaned) {
        seen = true;
        passed += caps[1].parse::<u32>().unwrap_or(0);
        failed += caps[2].parse::<u32>().unwrap_or(0);
        skipped += caps[3].parse::<u32>().unwrap_or(0);
        if let Some(d) = caps.get(6).and_then(|m| m.as_str().parse::<f64>().ok()) {
            duration_ms += (d * 1000.0) as u64;
        }
    }
    if !seen {
        return ParseResult::Failed {
            reason: "cargo test: no `test result:` line".to_string(),
            passthrough: input.to_string(),
        };
    }

    let failures = extract_failures(&cleaned);

    ParseResult::Full(TestResult {
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    })
}

/// Pull failing test names from the `failures:` block at the bottom of a
/// failing cargo-test run. Lines look like `    module::tests::test_name`.
fn extract_failures(cleaned: &str) -> Vec<TestFailure> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in cleaned.lines() {
        let trimmed = line.trim_end();
        if trimmed == "failures:" {
            in_block = !in_block;
            continue;
        }
        if in_block {
            let name = trimmed.trim();
            if name.is_empty() {
                in_block = false;
                continue;
            }
            // Skip header-style lines (e.g. "---- foo stdout ----") which
            // come BEFORE the bare-name listing.
            if name.starts_with("----") {
                continue;
            }
            // Skip indented panic/assert content (it starts with a non-name
            // character once we leave the bare-name listing).
            if name.contains(' ') || name.contains(':') && !name.contains("::") {
                continue;
            }
            out.push(TestFailure {
                name: name.to_string(),
                file: None,
                line: None,
                message: String::new(),
                snippet: None,
            });
        }
    }
    out
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["cargo".to_string(), "test".to_string()];
    let has_no_ff = args.iter().any(|a| a == "--no-fail-fast");
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("cargo") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) == Some("test") {
        user.remove(0);
    }
    argv.extend(user);
    if !has_no_ff {
        argv.push("--no-fail-fast".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig cargo_test {}", args.join(" "));
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
    fn parses_single_crate_summary() {
        let s = "
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s
";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 10);
                assert_eq!(r.failed, 0);
                assert_eq!(r.duration_ms, 420);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn aggregates_workspace_summaries() {
        let s = "
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: FAILED. 3 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out
";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 8);
                assert_eq!(r.failed, 2);
                assert_eq!(r.skipped, 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn extracts_failure_names() {
        let s = "
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out

failures:
    module::tests::test_one
    module::tests::test_two

";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.failures.len(), 2);
                assert_eq!(r.failures[0].name, "module::tests::test_one");
            }
            _ => panic!(),
        }
    }
}
