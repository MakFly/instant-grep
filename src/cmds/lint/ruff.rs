//! `ig ruff [args...]` — wrap `ruff check --output-format json`.

use anyhow::Result;
use serde::Deserialize;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{LintMessage, LintResult, ParseResult, TokenFormatter};

#[derive(Debug, Deserialize)]
struct RuffMsg {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    location: Option<RuffLoc>,
}

#[derive(Debug, Deserialize)]
struct RuffLoc {
    #[serde(default)]
    row: u32,
    #[serde(default)]
    column: u32,
}

pub fn parse(input: &str) -> ParseResult<LintResult> {
    let parsed: Option<Vec<RuffMsg>> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_array_str(input).and_then(|s| serde_json::from_str(s).ok()));
    let Some(msgs) = parsed else {
        // Empty array on a clean run is fine; but ruff exits 0 with stdout "[]"
        if input.trim() == "[]" {
            return ParseResult::Full(LintResult::default());
        }
        return ParseResult::Failed {
            reason: "ruff: no JSON array".to_string(),
            passthrough: input.to_string(),
        };
    };
    let mut errors = 0u32;
    let mut out = Vec::new();
    for m in &msgs {
        errors += 1; // ruff treats every diagnostic as an error by default
        let (line, col) = match &m.location {
            Some(l) => (l.row, l.column),
            None => (0, 0),
        };
        out.push(LintMessage {
            path: m.filename.clone(),
            line,
            col,
            rule: m.code.clone().unwrap_or_default(),
            message: m.message.clone(),
            severity: "error".to_string(),
        });
    }
    ParseResult::Full(LintResult {
        errors,
        warnings: 0,
        files: out,
    })
}

fn extract_json_array_str(input: &str) -> Option<&str> {
    let bytes = input.as_bytes();
    let start = bytes.iter().position(|&b| b == b'[')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&input[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["ruff".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("ruff") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) != Some("check") {
        argv.push("check".to_string());
    }
    let has_fmt = user.iter().any(|a| a.starts_with("--output-format"));
    argv.extend(user);
    if !has_fmt {
        argv.push("--output-format".to_string());
        argv.push("json".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig ruff {}", args.join(" "));
    let (output, outcome) = match parse(&run.stdout) {
        ParseResult::Full(r) => (
            TokenFormatter::new().format_lint_result(&r, opts.format_mode),
            ParseOutcome::Full,
        ),
        ParseResult::Partial { value, warning } => {
            let mut s = TokenFormatter::new().format_lint_result(&value, opts.format_mode);
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
    fn parses_ruff_json() {
        let s = r#"[{"code":"E501","message":"line too long","filename":"a.py","location":{"row":3,"column":1}}]"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.errors, 1);
                assert_eq!(r.files[0].rule, "E501");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn empty_array_is_full() {
        match parse("[]") {
            ParseResult::Full(r) => assert_eq!(r.errors, 0),
            _ => panic!(),
        }
    }
}
