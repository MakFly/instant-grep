//! `ig gt <subcmd>` — Graphite CLI wrapper. Surface stack summary as
//! one line per branch. Unknown subcommands fall through to passthrough.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig gt <subcommand> [args...]");
    }
    let sub = args[0].as_str();
    match sub {
        "log" | "branch" | "checkout" | "ls" => filtered(args),
        _ => filtered(args),
    }
}

fn filtered(args: &[String]) -> Result<i32> {
    let mut argv = vec!["gt".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig gt {}", args.join(" "));
    let out = if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        condense_stack(&run.merged)
    };
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

/// `gt log`/`gt log short` output is already roughly one line per branch
/// with extra ASCII art. We strip pure-decoration lines (◯, ◉, vertical
/// bars only) and keep meaningful ones.
pub fn condense_stack(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let meaningful: String = trimmed
            .chars()
            .filter(|c| !matches!(c, '│' | '|' | '◯' | '◉' | '┃' | '╿' | '╽'))
            .collect();
        if meaningful.trim().is_empty() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        return String::from("(empty)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condense_drops_decoration_lines() {
        let s = "│\nmain\n│\nfeat/x\n│\n";
        let out = condense_stack(s);
        assert!(out.contains("main"));
        assert!(out.contains("feat/x"));
        assert!(!out.contains("│\n│"));
    }

    #[test]
    fn condense_empty() {
        assert!(condense_stack("").starts_with("(empty)"));
    }
}
