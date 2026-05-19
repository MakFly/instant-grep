//! `ig gh <subcmd> [args]` — GitHub CLI wrapper with JSON-driven compaction.
//!
//! Auto-injects `--json <fields>` when ig owns the args (no user `--json`
//! flag, no `--jq`, no `--template`). Parses the JSON output and renders a
//! compact summary. Unknown subcommands fall through to passthrough.

use anyhow::Result;
use serde_json::Value;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    markdown::filter_markdown_body,
    spawn::capture,
};
use crate::parser::FormatMode;

const PR_FIELDS: &str = "number,title,state,author,url,baseRefName,headRefName,isDraft,mergeable,reviewDecision,createdAt,updatedAt";
const PR_VIEW_FIELDS: &str = "number,title,state,author,url,baseRefName,headRefName,body,isDraft,mergeable,reviewDecision,createdAt,updatedAt,labels";
const ISSUE_FIELDS: &str = "number,title,state,author,url,labels,createdAt,updatedAt";
const ISSUE_VIEW_FIELDS: &str = "number,title,state,author,url,labels,createdAt,updatedAt,body";
const RUN_FIELDS: &str =
    "databaseId,name,status,conclusion,workflowName,headBranch,event,createdAt,updatedAt";

/// Entry point — invoked from `main.rs` for `ig gh <args>`.
pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig gh <subcommand> [args...]");
    }

    let sub = args[0].as_str();
    let rest = &args[1..];

    // Recognised subcommands route to a typed renderer; anything else falls
    // through to passthrough (with TOML filter as a safety net).
    match (sub, rest.first().map(|s| s.as_str())) {
        ("pr", Some("list")) => {
            dispatch_list(&["pr", "list"], &rest[1..], PR_FIELDS, render_pr_list, opts)
        }
        ("pr", Some("view")) => dispatch_view(
            &["pr", "view"],
            &rest[1..],
            PR_VIEW_FIELDS,
            render_pr_view,
            opts,
        ),
        ("issue", Some("list")) => dispatch_list(
            &["issue", "list"],
            &rest[1..],
            ISSUE_FIELDS,
            render_issue_list,
            opts,
        ),
        ("issue", Some("view")) => dispatch_view(
            &["issue", "view"],
            &rest[1..],
            ISSUE_VIEW_FIELDS,
            render_issue_view,
            opts,
        ),
        ("run", Some("list")) => dispatch_list(
            &["run", "list"],
            &rest[1..],
            RUN_FIELDS,
            render_run_list,
            opts,
        ),
        ("run", Some("view")) => passthrough(args, opts),
        ("repo", Some("view")) => passthrough(args, opts),
        _ => passthrough(args, opts),
    }
}

/// Should we inject `--json <fields>`? Only if no user-provided format flag.
fn ig_owns_args(args: &[String]) -> bool {
    !args
        .iter()
        .any(|a| a == "--json" || a == "--jq" || a == "-q" || a == "--template" || a == "-t")
}

fn dispatch_list(
    sub: &[&str],
    args: &[String],
    fields: &str,
    render: fn(&Value, FormatMode) -> String,
    opts: RunOptions,
) -> Result<i32> {
    let owns = ig_owns_args(args);
    let mut argv = vec!["gh".to_string()];
    for s in sub {
        argv.push((*s).to_string());
    }
    argv.extend(args.iter().cloned());
    if owns {
        argv.push("--json".to_string());
        argv.push(fields.to_string());
    }

    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig gh {}", args_label(sub, args));

    if !owns {
        // User provided their own format flag → don't try to parse, passthrough.
        let out = format!("{}\n", run.merged.trim_end());
        emit(&label, &run, &out, ParseOutcome::Passthrough);
        return Ok(run.exit_code);
    }

    match serde_json::from_str::<Value>(run.stdout.trim()) {
        Ok(v) => {
            let out = render(&v, opts.format_mode);
            emit(&label, &run, &out, ParseOutcome::Full);
        }
        Err(_) => match try_toml_filter(&argv, &run.merged) {
            Some(f) => emit(&label, &run, &f, ParseOutcome::Passthrough),
            None => {
                let out = format!("{}\n(parse fallback: passthrough)\n", run.merged);
                emit(&label, &run, &out, ParseOutcome::Passthrough);
            }
        },
    }
    Ok(run.exit_code)
}

fn dispatch_view(
    sub: &[&str],
    args: &[String],
    fields: &str,
    render: fn(&Value, FormatMode) -> String,
    opts: RunOptions,
) -> Result<i32> {
    let owns = ig_owns_args(args);
    let mut argv = vec!["gh".to_string()];
    for s in sub {
        argv.push((*s).to_string());
    }
    argv.extend(args.iter().cloned());
    if owns {
        argv.push("--json".to_string());
        argv.push(fields.to_string());
    }

    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig gh {}", args_label(sub, args));

    if !owns {
        let out = format!("{}\n", run.merged.trim_end());
        emit(&label, &run, &out, ParseOutcome::Passthrough);
        return Ok(run.exit_code);
    }

    match serde_json::from_str::<Value>(run.stdout.trim()) {
        Ok(v) => {
            let out = render(&v, opts.format_mode);
            emit(&label, &run, &out, ParseOutcome::Full);
        }
        Err(_) => match try_toml_filter(&argv, &run.merged) {
            Some(f) => emit(&label, &run, &f, ParseOutcome::Passthrough),
            None => {
                let out = format!("{}\n(parse fallback: passthrough)\n", run.merged);
                emit(&label, &run, &out, ParseOutcome::Passthrough);
            }
        },
    }
    Ok(run.exit_code)
}

fn passthrough(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["gh".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig gh {}", args.join(" "));
    let out = if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        run.merged.clone()
    };
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

fn args_label(sub: &[&str], args: &[String]) -> String {
    let mut s = sub.join(" ");
    if !args.is_empty() {
        s.push(' ');
        s.push_str(&args.join(" "));
    }
    s
}

// ---------- Renderers ----------

pub fn render_pr_list(v: &Value, mode: FormatMode) -> String {
    let arr = match v.as_array() {
        Some(a) => a,
        None => return String::from("(no PRs)\n"),
    };
    if arr.is_empty() {
        return String::from("(no PRs)\n");
    }
    let mut out = String::new();
    out.push_str(&format!("{} PR(s):\n", arr.len()));
    for pr in arr {
        let n = pr.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
        let title = pr.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let state = pr.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let author = pr
            .get("author")
            .and_then(|x| x.get("login"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        let draft = pr.get("isDraft").and_then(|x| x.as_bool()).unwrap_or(false);
        let head = pr
            .get("headRefName")
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        let base = pr
            .get("baseRefName")
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        match mode {
            FormatMode::Ultra => {
                out.push_str(&format!("#{} {} [{}]\n", n, truncate(title, 60), state))
            }
            FormatMode::Compact => out.push_str(&format!(
                "#{} {}{} [{}] @{} ({}→{})\n",
                n,
                if draft { "DRAFT " } else { "" },
                truncate(title, 80),
                state,
                author,
                head,
                base
            )),
        }
    }
    out
}

pub fn render_pr_view(v: &Value, mode: FormatMode) -> String {
    let n = v.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    let author = v
        .get("author")
        .and_then(|x| x.get("login"))
        .and_then(|x| x.as_str())
        .unwrap_or("?");
    let head = v.get("headRefName").and_then(|x| x.as_str()).unwrap_or("?");
    let base = v.get("baseRefName").and_then(|x| x.as_str()).unwrap_or("?");
    let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("");
    let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("");
    let review = v
        .get("reviewDecision")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let mergeable = v.get("mergeable").and_then(|x| x.as_str()).unwrap_or("");
    let mut out = format!(
        "#{} {} [{}]\nauthor: @{}  branch: {} → {}\n",
        n, title, state, author, head, base
    );
    if !review.is_empty() {
        out.push_str(&format!("review: {}  mergeable: {}\n", review, mergeable));
    }
    if !url.is_empty() {
        out.push_str(&format!("url: {}\n", url));
    }
    if mode != FormatMode::Ultra && !body.is_empty() {
        let filtered = filter_markdown_body(body);
        if !filtered.is_empty() {
            out.push('\n');
            out.push_str(&filtered);
            out.push('\n');
        }
    }
    out
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
        let n = it.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
        let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let state = it.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let author = it
            .get("author")
            .and_then(|x| x.get("login"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        match mode {
            FormatMode::Ultra => {
                out.push_str(&format!("#{} {} [{}]\n", n, truncate(title, 60), state))
            }
            FormatMode::Compact => {
                let labels = labels_str(it.get("labels"));
                out.push_str(&format!(
                    "#{} {} [{}] @{}{}\n",
                    n,
                    truncate(title, 80),
                    state,
                    author,
                    if labels.is_empty() {
                        String::new()
                    } else {
                        format!(" {{{}}}", labels)
                    }
                ));
            }
        }
    }
    out
}

pub fn render_issue_view(v: &Value, mode: FormatMode) -> String {
    let n = v.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    let author = v
        .get("author")
        .and_then(|x| x.get("login"))
        .and_then(|x| x.as_str())
        .unwrap_or("?");
    let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("");
    let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("");
    let labels = labels_str(v.get("labels"));
    let mut out = format!("#{} {} [{}]\nauthor: @{}\n", n, title, state, author);
    if !labels.is_empty() {
        out.push_str(&format!("labels: {}\n", labels));
    }
    if !url.is_empty() {
        out.push_str(&format!("url: {}\n", url));
    }
    if mode != FormatMode::Ultra && !body.is_empty() {
        let filtered = filter_markdown_body(body);
        if !filtered.is_empty() {
            out.push('\n');
            out.push_str(&filtered);
            out.push('\n');
        }
    }
    out
}

pub fn render_run_list(v: &Value, mode: FormatMode) -> String {
    let arr = match v.as_array() {
        Some(a) => a,
        None => return String::from("(no runs)\n"),
    };
    if arr.is_empty() {
        return String::from("(no runs)\n");
    }
    let mut out = format!("{} run(s):\n", arr.len());
    for r in arr {
        let id = r.get("databaseId").and_then(|x| x.as_i64()).unwrap_or(0);
        let workflow = r.get("workflowName").and_then(|x| x.as_str()).unwrap_or("");
        let status = r.get("status").and_then(|x| x.as_str()).unwrap_or("");
        let conclusion = r.get("conclusion").and_then(|x| x.as_str()).unwrap_or("");
        let branch = r.get("headBranch").and_then(|x| x.as_str()).unwrap_or("?");
        match mode {
            FormatMode::Ultra => out.push_str(&format!(
                "{} {} [{}/{}]\n",
                id,
                truncate(workflow, 40),
                status,
                conclusion
            )),
            FormatMode::Compact => out.push_str(&format!(
                "{} {} [{}/{}] {}\n",
                id,
                truncate(workflow, 50),
                status,
                conclusion,
                branch
            )),
        }
    }
    out
}

fn labels_str(v: Option<&Value>) -> String {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return String::new();
    };
    arr.iter()
        .filter_map(|l| l.get("name").and_then(|x| x.as_str()))
        .collect::<Vec<_>>()
        .join(",")
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
    fn renders_pr_list_compact() {
        let v = json!([
            {"number": 12, "title": "Fix typo", "state": "OPEN", "author": {"login": "alice"}, "isDraft": false, "headRefName": "fix", "baseRefName": "main"}
        ]);
        let out = render_pr_list(&v, FormatMode::Compact);
        assert!(out.contains("#12"));
        assert!(out.contains("Fix typo"));
        assert!(out.contains("OPEN"));
        assert!(out.contains("@alice"));
        assert!(out.contains("fix→main"));
    }

    #[test]
    fn renders_pr_list_ultra() {
        let v = json!([{"number": 5, "title": "X", "state": "MERGED", "author": {"login": "bob"}}]);
        let out = render_pr_list(&v, FormatMode::Ultra);
        assert!(out.contains("#5"));
        assert!(out.contains("[MERGED]"));
    }

    #[test]
    fn renders_empty_pr_list() {
        let v = json!([]);
        assert!(render_pr_list(&v, FormatMode::Compact).contains("no PRs"));
    }

    #[test]
    fn renders_pr_view_with_body_filtered() {
        let v = json!({
            "number": 1,
            "title": "Feat",
            "state": "OPEN",
            "author": {"login": "z"},
            "headRefName": "f",
            "baseRefName": "main",
            "url": "https://x/y/pull/1",
            "body": "summary\n<!-- skip me -->\nend"
        });
        let out = render_pr_view(&v, FormatMode::Compact);
        assert!(out.contains("summary"));
        assert!(out.contains("end"));
        assert!(!out.contains("skip me"));
    }

    #[test]
    fn pr_view_ultra_omits_body() {
        let v = json!({"number": 1, "title": "T", "state": "OPEN", "author": {"login":"z"}, "headRefName":"f", "baseRefName":"m", "body":"long body here"});
        let out = render_pr_view(&v, FormatMode::Ultra);
        assert!(!out.contains("long body"));
    }

    #[test]
    fn renders_issue_list() {
        let v = json!([
            {"number": 7, "title": "Bug", "state": "OPEN", "author": {"login": "x"}, "labels": [{"name": "bug"}, {"name": "p1"}]}
        ]);
        let out = render_issue_list(&v, FormatMode::Compact);
        assert!(out.contains("#7"));
        assert!(out.contains("bug,p1"));
    }

    #[test]
    fn renders_run_list() {
        let v = json!([{"databaseId": 9999, "workflowName": "CI", "status": "completed", "conclusion": "success", "headBranch": "main"}]);
        let out = render_run_list(&v, FormatMode::Compact);
        assert!(out.contains("9999"));
        assert!(out.contains("CI"));
        assert!(out.contains("completed/success"));
    }

    #[test]
    fn ig_owns_args_detects_user_json() {
        assert!(!ig_owns_args(&["--json".to_string(), "x".to_string()]));
        assert!(ig_owns_args(&["--limit".to_string(), "10".to_string()]));
    }
}
