//! `ig tsc [args...]` — wrap `tsc --noEmit` and parse its standard
//! `path/file.ts(L,C): error TSxxxx: msg` diagnostic format.

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
        r"^([^():\n]+(?:\.[a-zA-Z0-9]+))\((\d+),(\d+)\):\s+(error|warning)\s+(TS\d+):\s+(.*)$",
    )
    .unwrap();
    let mut errors = 0u32;
    let mut warnings = 0u32;
    let mut out = Vec::new();
    for line in cleaned.lines() {
        if let Some(c) = re.captures(line.trim_end()) {
            let sev = &c[4];
            if sev == "error" {
                errors += 1;
            } else {
                warnings += 1;
            }
            out.push(LintMessage {
                path: c[1].to_string(),
                line: c[2].parse().unwrap_or(0),
                col: c[3].parse().unwrap_or(0),
                rule: c[5].to_string(),
                message: c[6].to_string(),
                severity: sev.to_string(),
            });
        }
    }
    if out.is_empty() {
        // Look for the "Found N errors" summary so we don't return Failed
        // on a clean run.
        let sum_re = Regex::new(r"Found\s+(\d+)\s+errors?").unwrap();
        if let Some(c) = sum_re.captures(&cleaned) {
            let n: u32 = c[1].parse().unwrap_or(0);
            return ParseResult::Full(LintResult {
                errors: n,
                warnings: 0,
                files: vec![],
            });
        }
        // Clean run: empty output.
        if cleaned.trim().is_empty() {
            return ParseResult::Full(LintResult::default());
        }
        return ParseResult::Failed {
            reason: "tsc: no diagnostics or summary found".to_string(),
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
    let mut argv = vec!["tsc".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("tsc") {
        user.remove(0);
    }
    let has_noemit = user.iter().any(|a| a == "--noEmit");
    let has_pretty = user.iter().any(|a| a == "--pretty");
    argv.extend(user);
    if !has_noemit {
        argv.push("--noEmit".to_string());
    }
    if !has_pretty {
        argv.push("--pretty".to_string());
        argv.push("false".to_string());
    }
    argv
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let argv = build_argv(args);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig tsc {}", args.join(" "));
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
    fn parses_tsc_errors() {
        let s = "\
src/a.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.
src/b.ts(3,1): error TS2304: Cannot find name 'foo'.
Found 2 errors in 2 files.
";
        match parse(s) {
            ParseResult::Full(r) => {
                assert_eq!(r.errors, 2);
                assert_eq!(r.files.len(), 2);
                assert_eq!(r.files[0].rule, "TS2322");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn clean_run() {
        match parse("") {
            ParseResult::Full(r) => assert_eq!(r.errors, 0),
            _ => panic!(),
        }
    }
}
