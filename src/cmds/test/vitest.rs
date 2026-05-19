//! `ig vitest [args...]` — wrap `vitest` with its JSON reporter and
//! emit a token-compressed summary via `TokenFormatter`.

use anyhow::Result;
use serde::Deserialize;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome, extract_json_object,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{ParseResult, TestFailure, TestResult, TokenFormatter};

/// Vitest's `--reporter=json` output shape (subset we care about).
#[derive(Debug, Deserialize)]
struct VitestJson {
    #[serde(rename = "numTotalTests", default)]
    num_total: u32,
    #[serde(rename = "numPassedTests", default)]
    num_passed: u32,
    #[serde(rename = "numFailedTests", default)]
    num_failed: u32,
    #[serde(rename = "numPendingTests", default)]
    num_pending: u32,
    #[serde(rename = "testResults", default)]
    test_results: Vec<VitestFile>,
    #[serde(rename = "startTime", default)]
    start_time: Option<f64>,
    #[serde(rename = "endTime", default)]
    end_time: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct VitestFile {
    #[serde(default)]
    name: String,
    #[serde(rename = "assertionResults", default)]
    assertion_results: Vec<VitestTest>,
}

#[derive(Debug, Deserialize)]
struct VitestTest {
    #[serde(rename = "fullName", default)]
    full_name: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "failureMessages", default)]
    failure_messages: Vec<String>,
}

pub fn parse(input: &str) -> ParseResult<TestResult> {
    let json_str = match serde_json::from_str::<VitestJson>(input) {
        Ok(j) => Some(j),
        Err(_) => {
            extract_json_object(input).and_then(|s| serde_json::from_str::<VitestJson>(s).ok())
        }
    };
    match json_str {
        Some(j) => {
            let mut failures = Vec::new();
            for f in &j.test_results {
                for t in &f.assertion_results {
                    if t.status == "failed" {
                        failures.push(TestFailure {
                            name: t.full_name.clone(),
                            file: if f.name.is_empty() {
                                None
                            } else {
                                Some(f.name.clone())
                            },
                            line: None,
                            message: t.failure_messages.join("\n"),
                            snippet: None,
                        });
                    }
                }
            }
            let duration_ms = match (j.start_time, j.end_time) {
                (Some(s), Some(e)) if e > s => (e - s) as u64,
                _ => 0,
            };
            let _ = j.num_total;
            ParseResult::Full(TestResult {
                passed: j.num_passed,
                failed: j.num_failed,
                skipped: j.num_pending,
                duration_ms,
                failures,
            })
        }
        None => ParseResult::Failed {
            reason: "vitest: no parseable JSON in output".to_string(),
            passthrough: input.to_string(),
        },
    }
}

/// Build the argv that will actually be executed.
///
/// If the user did not pass any `--reporter=...` flag we inject one so we
/// get JSON we can parse. We also prepend `vitest` if the user omitted it
/// (allowing `ig vitest run path/to/test`).
fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(args.len() + 2);
    argv.push("vitest".to_string());
    let has_reporter = args.iter().any(|a| a.starts_with("--reporter"));
    let mut user_args: Vec<String> = args.to_vec();
    // If the first user arg is itself "vitest", drop the dup.
    if user_args.first().map(|s| s.as_str()) == Some("vitest") {
        user_args.remove(0);
    }
    argv.extend(user_args);
    if !has_reporter {
        argv.push("--reporter=json".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig vitest {}", args.join(" "));

    let (output, outcome) = render(&run.stdout, &run.merged, &argv, opts);
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

fn render(stdout: &str, merged: &str, argv: &[String], opts: RunOptions) -> (String, ParseOutcome) {
    match parse(stdout) {
        ParseResult::Full(r) => (
            TokenFormatter::new().format_test_result(&r, opts.format_mode),
            ParseOutcome::Full,
        ),
        ParseResult::Partial { value, warning } => {
            let mut s = TokenFormatter::new().format_test_result(&value, opts.format_mode);
            s.push_str(&format!("\n(partial parse: {})\n", warning));
            (s, ParseOutcome::Partial)
        }
        ParseResult::Failed { passthrough, .. } => match try_toml_filter(argv, merged) {
            Some(f) => (f, ParseOutcome::Passthrough),
            None => (
                format!("{}\n(parse fallback: passthrough)\n", passthrough),
                ParseOutcome::Passthrough,
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"numTotalTests":3,"numPassedTests":2,"numFailedTests":1,"numPendingTests":0,"testResults":[{"name":"tests/a.test.ts","assertionResults":[{"fullName":"adds","status":"passed","failureMessages":[]},{"fullName":"subtracts","status":"failed","failureMessages":["expected 0 to be 1"]},{"fullName":"divides","status":"passed","failureMessages":[]}]}],"startTime":1000.0,"endTime":1420.0}"#;

    #[test]
    fn parses_full_json() {
        match parse(JSON) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 2);
                assert_eq!(r.failed, 1);
                assert_eq!(r.failures.len(), 1);
                assert_eq!(r.duration_ms, 420);
            }
            _ => panic!("expected Full"),
        }
    }

    #[test]
    fn handles_dotenv_prefix() {
        let prefixed = format!(
            "[dotenv@] loaded .env\n> app@1 test\n> vitest --reporter=json\n\n{}\n",
            JSON
        );
        match parse(&prefixed) {
            ParseResult::Full(r) => assert_eq!(r.failed, 1),
            other => panic!(
                "expected Full, got {:?}",
                matches!(other, ParseResult::Failed { .. })
            ),
        }
    }

    #[test]
    fn injects_reporter_when_missing() {
        let argv = build_argv(&[]);
        assert!(argv.contains(&"--reporter=json".to_string()));
    }

    #[test]
    fn keeps_user_reporter_flag() {
        let argv = build_argv(&["--reporter=verbose".to_string()]);
        assert_eq!(
            argv.iter().filter(|a| a.starts_with("--reporter")).count(),
            1
        );
    }

    #[test]
    fn drops_duplicate_vitest_token() {
        let argv = build_argv(&["vitest".to_string(), "run".to_string()]);
        assert_eq!(argv[0], "vitest");
        assert_eq!(argv.iter().filter(|a| a.as_str() == "vitest").count(), 1);
    }

    #[test]
    fn failed_parse_passthrough() {
        match parse("not json at all") {
            ParseResult::Failed { .. } => {}
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn golden_compact_matches_fixture() {
        use crate::parser::FormatMode;
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vitest/raw.txt"
        ))
        .unwrap();
        let expected = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vitest/expected.compact.txt"
        ))
        .unwrap();
        let r = match parse(&raw) {
            ParseResult::Full(r) => r,
            _ => panic!("parse must succeed on fixture"),
        };
        let got = TokenFormatter::new().format_test_result(&r, FormatMode::Compact);
        assert_eq!(got, expected);
    }

    #[test]
    fn golden_ultra_matches_fixture() {
        use crate::parser::FormatMode;
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vitest/raw.txt"
        ))
        .unwrap();
        let expected = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vitest/expected.ultra.txt"
        ))
        .unwrap();
        let r = match parse(&raw) {
            ParseResult::Full(r) => r,
            _ => panic!("parse must succeed on fixture"),
        };
        let got = TokenFormatter::new().format_test_result(&r, FormatMode::Ultra);
        assert_eq!(got, expected);
    }
}
