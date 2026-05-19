//! `ig rubocop [args...]` — wrap `rubocop --format json`.

use anyhow::Result;
use serde::Deserialize;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome, extract_json_object,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{LintMessage, LintResult, ParseResult, TokenFormatter};

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(default)]
    files: Vec<RFile>,
    #[serde(default)]
    summary: Summary,
}

#[derive(Debug, Default, Deserialize)]
struct Summary {
    #[serde(default)]
    offense_count: u32,
}

#[derive(Debug, Deserialize)]
struct RFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    offenses: Vec<Off>,
}

#[derive(Debug, Deserialize)]
struct Off {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    cop_name: String,
    #[serde(default)]
    location: Option<Loc>,
}

#[derive(Debug, Deserialize)]
struct Loc {
    #[serde(default)]
    line: u32,
    #[serde(default)]
    column: u32,
}

pub fn parse(input: &str) -> ParseResult<LintResult> {
    let parsed: Option<Report> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    let Some(r) = parsed else {
        return ParseResult::Failed {
            reason: "rubocop: no JSON".to_string(),
            passthrough: input.to_string(),
        };
    };
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for f in &r.files {
        for o in &f.offenses {
            let sev_l = o.severity.to_lowercase();
            let n = if sev_l == "error" || sev_l == "fatal" {
                errors += 1;
                "error"
            } else {
                warnings += 1;
                "warning"
            };
            let (line, col) = match &o.location {
                Some(l) => (l.line, l.column),
                None => (0, 0),
            };
            out.push(LintMessage {
                path: f.path.clone(),
                line,
                col,
                rule: o.cop_name.clone(),
                message: o.message.clone(),
                severity: n.to_string(),
            });
        }
    }
    let _ = r.summary.offense_count;
    ParseResult::Full(LintResult {
        errors,
        warnings,
        files: out,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["rubocop".to_string()];
    let has_fmt = args
        .iter()
        .any(|a| a == "--format" || a.starts_with("--format="));
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("rubocop") {
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
    let label = format!("ig rubocop {}", args.join(" "));
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
    fn parses_rubocop() {
        let s = r#"{"files":[{"path":"a.rb","offenses":[{"severity":"convention","message":"bad","cop_name":"Style/Foo","location":{"line":1,"column":2}}]}],"summary":{"offense_count":1}}"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.warnings, 1);
                assert_eq!(r.files[0].rule, "Style/Foo");
            }
            _ => panic!(),
        }
    }
}
