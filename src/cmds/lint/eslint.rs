//! `ig eslint [args...]` — wrap `eslint --format json` and emit a
//! compact `LintResult`.

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
struct EslintFile {
    #[serde(rename = "filePath")]
    file_path: String,
    #[serde(default)]
    messages: Vec<EslintMsg>,
}

#[derive(Debug, Deserialize)]
struct EslintMsg {
    #[serde(default)]
    severity: u32,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    column: u32,
    #[serde(default, rename = "ruleId")]
    rule_id: Option<String>,
    #[serde(default)]
    message: String,
}

pub fn parse(input: &str) -> ParseResult<LintResult> {
    // ESLint JSON output is an array (`[ {filePath, messages: [...]}, ... ]`).
    let parsed: Option<Vec<EslintFile>> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_array(input).and_then(|s| serde_json::from_str(s).ok()));
    let Some(files) = parsed else {
        return ParseResult::Failed {
            reason: "eslint: no JSON array".to_string(),
            passthrough: input.to_string(),
        };
    };
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for f in &files {
        for m in &f.messages {
            let sev = match m.severity {
                2 => {
                    errors += 1;
                    "error"
                }
                1 => {
                    warnings += 1;
                    "warning"
                }
                _ => "info",
            };
            out.push(LintMessage {
                path: f.file_path.clone(),
                line: m.line,
                col: m.column,
                rule: m.rule_id.clone().unwrap_or_default(),
                message: m.message.clone(),
                severity: sev.to_string(),
            });
        }
    }
    ParseResult::Full(LintResult {
        errors,
        warnings,
        files: out,
    })
}

fn extract_json_array(input: &str) -> Option<&str> {
    // Find first `[`, then a balanced bracket scan (cheap variant).
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
    let mut argv = vec!["eslint".to_string()];
    let has_fmt = args
        .iter()
        .any(|a| a == "--format" || a.starts_with("--format="));
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("eslint") {
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
    let label = format!("ig eslint {}", args.join(" "));
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

    const JSON: &str = r#"[{"filePath":"/p/a.ts","messages":[{"ruleId":"no-unused","severity":1,"message":"unused","line":3,"column":7},{"ruleId":"semi","severity":2,"message":"missing semi","line":5,"column":1}]}]"#;

    #[test]
    fn parses_basic_eslint() {
        match parse(JSON) {
            ParseResult::Full(r) => {
                assert_eq!(r.errors, 1);
                assert_eq!(r.warnings, 1);
                assert_eq!(r.files.len(), 2);
                assert_eq!(r.files[0].rule, "no-unused");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn extract_array_from_noise() {
        let s = format!("warning: deprecated\n{}\n", JSON);
        match parse(&s) {
            ParseResult::Full(r) => assert_eq!(r.errors + r.warnings, 2),
            _ => panic!(),
        }
    }
}
