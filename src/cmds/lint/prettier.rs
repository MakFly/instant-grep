//! `ig prettier [args...]` — wrap `prettier --check` and surface only the
//! files that need formatting.

use anyhow::Result;

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
    let mut out = Vec::new();
    for line in cleaned.lines() {
        let trimmed = line.trim();
        // `[warn] src/foo.ts` is what `prettier --check` emits per offending file.
        if let Some(rest) = trimmed.strip_prefix("[warn] ") {
            out.push(LintMessage {
                path: rest.to_string(),
                line: 0,
                col: 0,
                rule: "format".to_string(),
                message: "needs format".to_string(),
                severity: "warning".to_string(),
            });
        }
    }
    let warnings = out.len() as u32;
    if warnings == 0 && !cleaned.contains("All matched files use Prettier code style") {
        // Don't fail on a clean run; just emit empty result.
        if cleaned.trim().is_empty() {
            return ParseResult::Full(LintResult::default());
        }
    }
    ParseResult::Full(LintResult {
        errors: 0,
        warnings,
        files: out,
    })
}

fn build_argv(args: &[String]) -> Vec<String> {
    let mut argv = vec!["prettier".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("prettier") {
        user.remove(0);
    }
    let has_check = user.iter().any(|a| a == "--check" || a == "-c");
    argv.extend(user);
    if !has_check {
        argv.push("--check".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig prettier {}", args.join(" "));
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
    fn parses_warn_lines() {
        let s = "Checking formatting...\n[warn] src/a.ts\n[warn] src/b.ts\n";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.warnings, 2);
                assert_eq!(r.files.len(), 2);
            }
            _ => panic!(),
        }
    }
}
