//! `ig rspec [args...]` — wrap `rspec --format json`.

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
struct RspecJson {
    #[serde(default)]
    examples: Vec<RspecExample>,
    #[serde(default)]
    summary: RspecSummary,
}

#[derive(Debug, Default, Deserialize)]
struct RspecSummary {
    #[serde(default)]
    example_count: u32,
    #[serde(default)]
    failure_count: u32,
    #[serde(default)]
    pending_count: u32,
    #[serde(default)]
    duration: f64,
}

#[derive(Debug, Deserialize)]
struct RspecExample {
    #[serde(default)]
    description: String,
    #[serde(default)]
    full_description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    file_path: String,
    #[serde(default)]
    line_number: u32,
    #[serde(default)]
    exception: Option<RspecExc>,
}

#[derive(Debug, Deserialize)]
struct RspecExc {
    #[serde(default)]
    message: String,
}

pub fn parse(input: &str) -> ParseResult<TestResult> {
    let json: Option<RspecJson> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    match json {
        Some(j) => {
            let mut failures = Vec::new();
            for ex in &j.examples {
                if ex.status == "failed" {
                    let name = if !ex.full_description.is_empty() {
                        ex.full_description.clone()
                    } else {
                        ex.description.clone()
                    };
                    failures.push(TestFailure {
                        name,
                        file: if ex.file_path.is_empty() {
                            None
                        } else {
                            Some(ex.file_path.clone())
                        },
                        line: if ex.line_number == 0 {
                            None
                        } else {
                            Some(ex.line_number)
                        },
                        message: ex
                            .exception
                            .as_ref()
                            .map(|e| e.message.clone())
                            .unwrap_or_default(),
                        snippet: None,
                    });
                }
            }
            let s = &j.summary;
            ParseResult::Full(TestResult {
                passed: s
                    .example_count
                    .saturating_sub(s.failure_count + s.pending_count),
                failed: s.failure_count,
                skipped: s.pending_count,
                duration_ms: (s.duration * 1000.0) as u64,
                failures,
            })
        }
        None => ParseResult::Failed {
            reason: "rspec: no JSON".to_string(),
            passthrough: input.to_string(),
        },
    }
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["rspec".to_string()];
    let has_fmt = args
        .iter()
        .any(|a| a == "--format" || a.starts_with("--format="));
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("rspec") {
        user.remove(0);
    }
    argv.extend(user);
    if !has_fmt {
        argv.push("--format".to_string());
        argv.push("json".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig rspec {}", args.join(" "));
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
    fn parses_basic_rspec_json() {
        let s = r#"{"examples":[{"description":"a","full_description":"thing a","status":"passed","file_path":"./spec/a.rb","line_number":3,"exception":null},{"description":"b","full_description":"thing b","status":"failed","file_path":"./spec/a.rb","line_number":7,"exception":{"message":"bad"}}],"summary":{"example_count":2,"failure_count":1,"pending_count":0,"duration":0.5}}"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.passed, 1);
                assert_eq!(r.failed, 1);
                assert_eq!(r.duration_ms, 500);
                assert_eq!(r.failures.len(), 1);
            }
            _ => panic!(),
        }
    }
}
