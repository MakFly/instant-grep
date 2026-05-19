//! `ig go_test [args...]` — wrap `go test -json ./...` and stream-parse NDJSON.

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome, extract_json_lines,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{ParseResult, TestFailure, TestResult, TokenFormatter};

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(rename = "Action")]
    action: String,
    #[serde(rename = "Package", default)]
    package: String,
    #[serde(rename = "Test", default)]
    test: String,
    #[serde(rename = "Output", default)]
    output: String,
    #[serde(rename = "Elapsed", default)]
    elapsed: f64,
}

pub fn parse(input: &str) -> ParseResult<TestResult> {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut duration_ms = 0u64;
    let mut failures: Vec<TestFailure> = Vec::new();
    // Pkg+Test → buffered output for failure messages.
    let mut buffers: HashMap<String, String> = HashMap::new();
    let mut saw_event = false;
    for line in extract_json_lines(input) {
        let Ok(ev) = serde_json::from_str::<Event>(line) else {
            continue;
        };
        saw_event = true;
        let key = format!("{}|{}", ev.package, ev.test);
        match ev.action.as_str() {
            "output" if !ev.test.is_empty() => {
                buffers.entry(key.clone()).or_default().push_str(&ev.output);
            }
            "pass" if !ev.test.is_empty() => {
                passed += 1;
                buffers.remove(&key);
            }
            "fail" if !ev.test.is_empty() => {
                failed += 1;
                let msg = buffers
                    .remove(&key)
                    .unwrap_or_default()
                    .lines()
                    .find(|l| {
                        let t = l.trim_start();
                        !t.is_empty()
                            && !t.starts_with("---")
                            && !t.starts_with("=== RUN")
                            && !t.starts_with("=== PAUSE")
                            && !t.starts_with("=== CONT")
                    })
                    .unwrap_or("")
                    .trim()
                    .to_string();
                failures.push(TestFailure {
                    name: format!("{}/{}", ev.package, ev.test),
                    file: None,
                    line: None,
                    message: msg,
                    snippet: None,
                });
            }
            "skip" if !ev.test.is_empty() => {
                skipped += 1;
                buffers.remove(&key);
            }
            "pass" | "fail" if ev.test.is_empty() => {
                // Package-level event; sum elapsed.
                duration_ms += (ev.elapsed * 1000.0) as u64;
            }
            _ => {}
        }
    }
    if !saw_event {
        return ParseResult::Failed {
            reason: "go test: no JSON events".to_string(),
            passthrough: input.to_string(),
        };
    }
    ParseResult::Full(TestResult {
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["go".to_string(), "test".to_string()];
    let has_json = args.iter().any(|a| a == "-json");
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("go") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) == Some("test") {
        user.remove(0);
    }
    argv.extend(user);
    if !has_json {
        argv.push("-json".to_string());
    }
    if args.iter().all(|a| !a.starts_with('.')) && args.iter().all(|a| !a.contains('/')) {
        argv.push("./...".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig go_test {}", args.join(" "));
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
    fn parses_ndjson_events() {
        let s = r#"
{"Action":"run","Package":"pkg/a","Test":"TestX"}
{"Action":"output","Package":"pkg/a","Test":"TestX","Output":"--- PASS: TestX (0.01s)\n"}
{"Action":"pass","Package":"pkg/a","Test":"TestX","Elapsed":0.01}
{"Action":"run","Package":"pkg/a","Test":"TestY"}
{"Action":"output","Package":"pkg/a","Test":"TestY","Output":"a_test.go:5: oops\n"}
{"Action":"fail","Package":"pkg/a","Test":"TestY","Elapsed":0.02}
{"Action":"pass","Package":"pkg/a","Elapsed":0.5}
"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 1);
                assert_eq!(r.failed, 1);
                assert_eq!(r.duration_ms, 500);
                assert_eq!(r.failures.len(), 1);
                assert!(r.failures[0].message.contains("oops"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn handles_interleaved_packages() {
        let s = r#"
{"Action":"run","Package":"pkg/a","Test":"T1"}
{"Action":"run","Package":"pkg/b","Test":"T1"}
{"Action":"pass","Package":"pkg/b","Test":"T1","Elapsed":0.01}
{"Action":"pass","Package":"pkg/a","Test":"T1","Elapsed":0.02}
"#;
        match parse(s) {
            ParseResult::Full(r) => assert_eq!(r.passed, 2),
            _ => panic!(),
        }
    }

    #[test]
    fn failed_parse_on_text_only() {
        match parse("PASS\nok      pkg     0.123s\n") {
            ParseResult::Failed { .. } => {}
            _ => panic!(),
        }
    }
}
