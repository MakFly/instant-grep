//! `ig biome [args...]` — wrap `biome check --reporter=json`.

use anyhow::Result;
use serde::Deserialize;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome, extract_json_object,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::{LintMessage, LintResult, ParseResult, TokenFormatter};

#[derive(Debug, Deserialize, Default)]
struct BiomeReport {
    #[serde(default)]
    diagnostics: Vec<BiomeDiag>,
    #[serde(default)]
    summary: Option<BiomeSummary>,
}

#[derive(Debug, Default, Deserialize)]
struct BiomeSummary {
    #[serde(default)]
    errors: u32,
    #[serde(default)]
    warnings: u32,
}

#[derive(Debug, Deserialize)]
struct BiomeDiag {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: Option<BiomeLoc>,
}

#[derive(Debug, Deserialize)]
struct BiomeLoc {
    #[serde(default)]
    path: Option<BiomePath>,
    #[serde(default)]
    span: Option<[u32; 2]>,
}

#[derive(Debug, Deserialize)]
struct BiomePath {
    #[serde(default)]
    file: Option<String>,
}

pub fn parse(input: &str) -> ParseResult<LintResult> {
    let parsed: Option<BiomeReport> = serde_json::from_str(input)
        .ok()
        .or_else(|| extract_json_object(input).and_then(|s| serde_json::from_str(s).ok()));
    let Some(report) = parsed else {
        return ParseResult::Failed {
            reason: "biome: no JSON".to_string(),
            passthrough: input.to_string(),
        };
    };
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for d in &report.diagnostics {
        let sev_l = d.severity.to_lowercase();
        let normalized = if sev_l.contains("error") || sev_l == "fatal" {
            errors += 1;
            "error"
        } else if sev_l.contains("warn") {
            warnings += 1;
            "warning"
        } else {
            "info"
        };
        let path = d
            .location
            .as_ref()
            .and_then(|l| l.path.as_ref())
            .and_then(|p| p.file.clone())
            .unwrap_or_default();
        out.push(LintMessage {
            path,
            line: 0,
            col: d
                .location
                .as_ref()
                .and_then(|l| l.span)
                .map(|s| s[0])
                .unwrap_or(0),
            rule: d.category.clone(),
            message: d.description.clone(),
            severity: normalized.to_string(),
        });
    }
    if let Some(s) = report.summary {
        // Prefer summary counts when present — diagnostics may be truncated.
        if s.errors > 0 || s.warnings > 0 {
            errors = s.errors;
            warnings = s.warnings;
        }
    }
    ParseResult::Full(LintResult {
        errors,
        warnings,
        files: out,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["biome".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("biome") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) != Some("check")
        && user.first().map(|s| s.as_str()) != Some("lint")
        && user.first().map(|s| s.as_str()) != Some("format")
    {
        argv.push("check".to_string());
    }
    let has_reporter = user.iter().any(|a| a.starts_with("--reporter"));
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
    let label = format!("ig biome {}", args.join(" "));
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
    fn parses_minimal_summary() {
        let s = r#"{"diagnostics":[],"summary":{"errors":3,"warnings":1}}"#;
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.errors, 3);
                assert_eq!(r.warnings, 1);
            }
            _ => panic!(),
        }
    }
}
