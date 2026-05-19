//! `ig glab <subcmd>` — GitLab CLI wrapper. Mirrors the `gh` surface but
//! against `glab` (mr/issue/pipeline). When `glab` lacks a `--json` flag for
//! the chosen subcommand we fall through to the TOML filter pipeline.

use anyhow::Result;
use serde_json::Value;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};
use crate::parser::FormatMode;

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig glab <subcommand> [args...]");
    }
    let sub = args[0].as_str();
    let rest = &args[1..];
    match (sub, rest.first().map(|s| s.as_str())) {
        ("mr", Some("list")) => json_list(&["mr", "list"], &rest[1..], render_mr_list, opts),
        ("mr", Some("view")) => json_view(&["mr", "view"], &rest[1..], render_mr_view, opts),
        ("issue", Some("list")) => {
            json_list(&["issue", "list"], &rest[1..], render_issue_list, opts)
        }
        ("issue", Some("view")) => {
            json_view(&["issue", "view"], &rest[1..], render_issue_view, opts)
        }
        ("pipeline", Some("list")) => passthrough(args, opts),
        ("pipeline", Some("view")) => passthrough(args, opts),
        _ => passthrough(args, opts),
    }
}

fn ig_owns(args: &[String]) -> bool {
    !args
        .iter()
        .any(|a| a == "-F" || a == "--output" || a == "-O")
}

fn json_list(
    sub: &[&str],
    args: &[String],
    render: fn(&Value, FormatMode) -> String,
    opts: RunOptions,
) -> Result<i32> {
    let owns = ig_owns(args);
    let mut argv = vec!["glab".to_string()];
    for s in sub {
        argv.push((*s).to_string());
    }
    argv.extend(args.iter().cloned());
    if owns {
        argv.push("-F".to_string());
        argv.push("json".to_string());
    }
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig glab {} {}", sub.join(" "), args.join(" "));
    if !owns {
        emit(
            &label,
            &run,
            &format!("{}\n", run.merged.trim_end()),
            ParseOutcome::Passthrough,
        );
        return Ok(run.exit_code);
    }
    match serde_json::from_str::<Value>(run.stdout.trim()) {
        Ok(v) => emit(
            &label,
            &run,
            &render(&v, opts.format_mode),
            ParseOutcome::Full,
        ),
        Err(_) => match try_toml_filter(&argv, &run.merged) {
            Some(f) => emit(&label, &run, &f, ParseOutcome::Passthrough),
            None => emit(&label, &run, &run.merged, ParseOutcome::Passthrough),
        },
    }
    Ok(run.exit_code)
}

fn json_view(
    sub: &[&str],
    args: &[String],
    render: fn(&Value, FormatMode) -> String,
    opts: RunOptions,
) -> Result<i32> {
    json_list(sub, args, render, opts)
}

fn passthrough(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["glab".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig glab {}", args.join(" "));
    let out = try_toml_filter(&argv, &run.merged).unwrap_or_else(|| run.merged.clone());
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

pub fn render_mr_list(v: &Value, mode: FormatMode) -> String {
    let arr = match v.as_array() {
        Some(a) => a,
        None => return String::from("(no MRs)\n"),
    };
    if arr.is_empty() {
        return String::from("(no MRs)\n");
    }
    let mut out = format!("{} MR(s):\n", arr.len());
    for mr in arr {
        let iid = mr.get("iid").and_then(|x| x.as_i64()).unwrap_or(0);
        let title = mr.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let state = mr.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let author = mr
            .get("author")
            .and_then(|a| a.get("username"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        match mode {
            FormatMode::Ultra => {
                out.push_str(&format!("!{} {} [{}]\n", iid, truncate(title, 60), state))
            }
            FormatMode::Compact => out.push_str(&format!(
                "!{} {} [{}] @{}\n",
                iid,
                truncate(title, 80),
                state,
                author
            )),
        }
    }
    out
}

pub fn render_mr_view(v: &Value, _mode: FormatMode) -> String {
    let iid = v.get("iid").and_then(|x| x.as_i64()).unwrap_or(0);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    format!("!{} {} [{}]\n", iid, title, state)
}

pub fn render_issue_list(v: &Value, mode: FormatMode) -> String {
    let arr = match v.as_array() {
        Some(a) => a,
        None => return String::from("(no issues)\n"),
    };
    if arr.is_empty() {
        return String::from("(no issues)\n");
    }
    let mut out = format!("{} issue(s):\n", arr.len());
    for it in arr {
        let iid = it.get("iid").and_then(|x| x.as_i64()).unwrap_or(0);
        let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let state = it.get("state").and_then(|x| x.as_str()).unwrap_or("");
        match mode {
            FormatMode::Ultra => {
                out.push_str(&format!("#{} {} [{}]\n", iid, truncate(title, 60), state))
            }
            FormatMode::Compact => {
                out.push_str(&format!("#{} {} [{}]\n", iid, truncate(title, 80), state))
            }
        }
    }
    out
}

pub fn render_issue_view(v: &Value, _mode: FormatMode) -> String {
    let iid = v.get("iid").and_then(|x| x.as_i64()).unwrap_or(0);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    format!("#{} {} [{}]\n", iid, title, state)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_mr_list() {
        let v = json!([{"iid": 3, "title": "Refactor", "state": "opened", "author": {"username": "z"}}]);
        let out = render_mr_list(&v, FormatMode::Compact);
        assert!(out.contains("!3"));
        assert!(out.contains("Refactor"));
        assert!(out.contains("@z"));
    }

    #[test]
    fn empty_mr_list() {
        assert!(render_mr_list(&json!([]), FormatMode::Compact).contains("no MRs"));
    }

    #[test]
    fn renders_issue_list() {
        let v = json!([{"iid": 1, "title": "Bug", "state": "opened"}]);
        let out = render_issue_list(&v, FormatMode::Compact);
        assert!(out.contains("#1"));
        assert!(out.contains("Bug"));
    }
}
