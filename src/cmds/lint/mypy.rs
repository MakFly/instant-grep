//! `ig mypy [args...]` — parse mypy's `file:line: error: msg [code]` text format.

use anyhow::Result;
use regex::Regex;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
    strip_ansi,
};
use crate::parser::{LintMessage, LintResult, ParseResult, TokenFormatter};

pub fn parse(input: &str) -> ParseResult<LintResult> {
    let cleaned = strip_ansi(input);
    let re = Regex::new(
        r"^([^\s:]+):(\d+):(?:(\d+):)?\s*(error|warning|note):\s*(.+?)(?:\s+\[([^\]]+)\])?$",
    )
    .unwrap();
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for line in cleaned.lines() {
        if let Some(c) = re.captures(line.trim_end()) {
            let sev = &c[4];
            let s_norm = match sev {
                "error" => {
                    errors += 1;
                    "error"
                }
                "warning" => {
                    warnings += 1;
                    "warning"
                }
                _ => "info",
            };
            out.push(LintMessage {
                path: c[1].to_string(),
                line: c[2].parse().unwrap_or(0),
                col: c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0),
                rule: c.get(6).map(|m| m.as_str().to_string()).unwrap_or_default(),
                message: c[5].to_string(),
                severity: s_norm.to_string(),
            });
        }
    }
    if out.is_empty() && cleaned.trim().is_empty() {
        return ParseResult::Full(LintResult::default());
    }
    if out.is_empty() && !cleaned.contains("Success: no issues") {
        return ParseResult::Failed {
            reason: "mypy: no diagnostics".to_string(),
            passthrough: input.to_string(),
        };
    }
    ParseResult::Full(LintResult {
        errors,
        warnings,
        files: out,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["mypy".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("mypy") {
        user.remove(0);
    }
    argv.extend(user);
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig mypy {}", args.join(" "));
    let (output, outcome) = match parse(&run.merged) {
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
    fn parses_mypy_errors() {
        let s = "src/a.py:12: error: Incompatible return value type [return-value]\nsrc/b.py:3:5: warning: foo\nFound 1 error in 1 file\n";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.errors, 1);
                assert_eq!(r.warnings, 1);
                assert_eq!(r.files[0].rule, "return-value");
            }
            _ => panic!(),
        }
    }
}
