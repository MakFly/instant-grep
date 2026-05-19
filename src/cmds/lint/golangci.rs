//! `ig golangci-lint [args...]` — wrap `golangci-lint run --out-format json`.

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
    #[serde(rename = "Issues", default)]
    issues: Vec<Issue>,
}

#[derive(Debug, Deserialize)]
struct Issue {
    #[serde(rename = "FromLinter", default)]
    from_linter: String,
    #[serde(rename = "Text", default)]
    text: String,
    #[serde(rename = "Severity", default)]
    severity: String,
    #[serde(rename = "Pos", default)]
    pos: Option<Pos>,
}

#[derive(Debug, Deserialize)]
struct Pos {
    #[serde(rename = "Filename", default)]
    filename: String,
    #[serde(rename = "Line", default)]
    line: u32,
    #[serde(rename = "Column", default)]
    column: u32,
}

pub fn parse(input: &str) -> ParseResult<LintResult> {
    let parsed: Option<Report> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    let Some(r) = parsed else {
        return ParseResult::Failed {
            reason: "golangci-lint: no JSON".to_string(),
            passthrough: input.to_string(),
        };
    };
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for i in &r.issues {
        let sev_l = i.severity.to_lowercase();
        let n = if sev_l.contains("error") {
            errors += 1;
            "error"
        } else {
            warnings += 1;
            "warning"
        };
        let (path, line, col) = match &i.pos {
            Some(p) => (p.filename.clone(), p.line, p.column),
            None => (String::new(), 0, 0),
        };
        out.push(LintMessage {
            path,
            line,
            col,
            rule: i.from_linter.clone(),
            message: i.text.clone(),
            severity: n.to_string(),
        });
    }
    ParseResult::Full(LintResult {
        errors,
        warnings,
        files: out,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["golangci-lint".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("golangci-lint") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) != Some("run") {
        argv.push("run".to_string());
    }
    let has_out = user.iter().any(|a| a.starts_with("--out-format"));
    argv.extend(user);
    if !has_out {
        argv.push("--out-format".to_string());
        argv.push("json".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig golangci-lint {}", args.join(" "));
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
    fn parses_golangci() {
        let s = r#"{"Issues":[{"FromLinter":"govet","Text":"shadow x","Severity":"warning","Pos":{"Filename":"a.go","Line":5,"Column":3}}]}"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.warnings, 1);
                assert_eq!(r.files[0].rule, "govet");
            }
            _ => panic!(),
        }
    }
}
