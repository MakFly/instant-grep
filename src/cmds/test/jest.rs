//! `ig jest [args...]` — wrap `jest --json` and emit a compact summary.

use anyhow::Result;
use serde::Deserialize;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome, extract_json_object,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{ParseResult, TestFailure, TestResult, TokenFormatter};

#[derive(Debug, Deserialize)]
struct JestJson {
    #[serde(rename = "numTotalTests", default)]
    num_total: u32,
    #[serde(rename = "numPassedTests", default)]
    num_passed: u32,
    #[serde(rename = "numFailedTests", default)]
    num_failed: u32,
    #[serde(rename = "numPendingTests", default)]
    num_pending: u32,
    #[serde(rename = "testResults", default)]
    test_results: Vec<JestFile>,
    #[serde(rename = "startTime", default)]
    start_time: Option<u64>,
    #[serde(rename = "endTime", default)]
    end_time: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct JestFile {
    #[serde(default)]
    name: String,
    #[serde(rename = "assertionResults", default)]
    assertion_results: Vec<JestTest>,
}

#[derive(Debug, Deserialize)]
struct JestTest {
    #[serde(rename = "fullName", default)]
    full_name: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "failureMessages", default)]
    failure_messages: Vec<String>,
}

pub fn parse(input: &str) -> ParseResult<TestResult> {
    let json: Option<JestJson> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    match json {
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
                (Some(s), Some(e)) if e > s => e - s,
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
            reason: "jest: no parseable JSON in output".to_string(),
            passthrough: input.to_string(),
        },
    }
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["jest".to_string()];
    let has_json = args.iter().any(|a| a == "--json");
    let mut user_args: Vec<String> = args.to_vec();
    if user_args.first().map(|s| s.as_str()) == Some("jest") {
        user_args.remove(0);
    }
    argv.extend(user_args);
    if !has_json {
        argv.push("--json".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig jest {}", args.join(" "));

    let (output, outcome) = match parse(&run.stdout) {
        ParseResult::Full(r) => (
            TokenFormatter::new().format_test_result(&r, opts.format_mode),
            ParseOutcome::Full,
        ),
        ParseResult::Partial { value, warning } => {
            let mut s = TokenFormatter::new().format_test_result(&value, opts.format_mode);
            s.push_str(&format!("\n(partial parse: {})\n", warning));
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
    const JSON: &str = r#"{"numTotalTests":2,"numPassedTests":1,"numFailedTests":1,"numPendingTests":0,"testResults":[{"name":"a.test.js","assertionResults":[{"fullName":"a","status":"passed","failureMessages":[]},{"fullName":"b","status":"failed","failureMessages":["bad"]}]}],"startTime":1000,"endTime":1100}"#;

    #[test]
    fn parses_jest_json() {
        match parse(JSON) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 1);
                assert_eq!(r.failed, 1);
                assert_eq!(r.duration_ms, 100);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn injects_json_flag() {
        let argv = build_argv(&[]);
        assert!(argv.contains(&"--json".to_string()));
    }
}
