//! `ig playwright [args...]` — wrap `playwright test --reporter=json`.

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
struct PwJson {
    #[serde(default)]
    stats: PwStats,
    #[serde(default)]
    suites: Vec<PwSuite>,
}

#[derive(Debug, Default, Deserialize)]
struct PwStats {
    #[serde(default)]
    expected: u32,
    #[serde(default)]
    unexpected: u32,
    #[serde(default)]
    flaky: u32,
    #[serde(default)]
    skipped: u32,
    #[serde(default)]
    duration: f64,
}

#[derive(Debug, Default, Deserialize)]
struct PwSuite {
    #[serde(default)]
    file: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    specs: Vec<PwSpec>,
    #[serde(default)]
    suites: Vec<PwSuite>,
}

#[derive(Debug, Default, Deserialize)]
struct PwSpec {
    #[serde(default)]
    title: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    file: String,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    tests: Vec<PwTest>,
}

#[derive(Debug, Default, Deserialize)]
struct PwTest {
    #[serde(default)]
    results: Vec<PwResult>,
}

#[derive(Debug, Default, Deserialize)]
struct PwResult {
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<PwError>,
}

#[derive(Debug, Default, Deserialize)]
struct PwError {
    #[serde(default)]
    message: String,
}

fn collect_failures(suites: &[PwSuite], parent_file: &str, out: &mut Vec<TestFailure>) {
    for s in suites {
        let file = if s.file.is_empty() {
            parent_file.to_string()
        } else {
            s.file.clone()
        };
        for spec in &s.specs {
            if !spec.ok {
                let msg = spec
                    .tests
                    .iter()
                    .flat_map(|t| t.results.iter())
                    .find_map(|r| r.error.as_ref().map(|e| e.message.clone()))
                    .unwrap_or_default();
                let spec_file = if spec.file.is_empty() {
                    file.clone()
                } else {
                    spec.file.clone()
                };
                out.push(TestFailure {
                    name: spec.title.clone(),
                    file: if spec_file.is_empty() {
                        None
                    } else {
                        Some(spec_file)
                    },
                    line: if spec.line == 0 {
                        None
                    } else {
                        Some(spec.line)
                    },
                    message: msg,
                    snippet: None,
                });
            }
        }
        collect_failures(&s.suites, &file, out);
    }
}

pub fn parse(input: &str) -> ParseResult<TestResult> {
    let json: Option<PwJson> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    match json {
        Some(j) => {
            let mut failures = Vec::new();
            collect_failures(&j.suites, "", &mut failures);
            ParseResult::Full(TestResult {
                passed: j.stats.expected,
                failed: j.stats.unexpected + j.stats.flaky,
                skipped: j.stats.skipped,
                duration_ms: j.stats.duration as u64,
                failures,
            })
        }
        None => ParseResult::Failed {
            reason: "playwright: no JSON".to_string(),
            passthrough: input.to_string(),
        },
    }
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["playwright".to_string(), "test".to_string()];
    let has_reporter = args.iter().any(|a| a.starts_with("--reporter"));
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("playwright") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) == Some("test") {
        user.remove(0);
    }
    argv.extend(user);
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
    let label = format!("ig playwright {}", args.join(" "));
    let (output, outcome) = match parse(&run.stdout) {
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
    fn parses_minimal() {
        let s = r#"{"stats":{"expected":2,"unexpected":1,"flaky":0,"skipped":0,"duration":1234.5},"suites":[]}"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 2);
                assert_eq!(r.failed, 1);
                assert_eq!(r.duration_ms, 1234);
            }
            _ => panic!(),
        }
    }
}
