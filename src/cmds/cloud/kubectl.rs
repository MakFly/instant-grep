//! `ig kubectl <verb> [args]` — kubectl wrapper restricted to a small set of
//! readable verbs (`get`, `logs`, `describe`, `apply`). Everything else is
//! passthrough so interactive verbs (exec, port-forward, edit) are unchanged.

use anyhow::Result;
use serde_json::Value;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig kubectl <verb> [args...]");
    }
    let verb = args[0].as_str();
    match verb {
        "get" => run_get(args),
        "logs" => run_logs(args),
        "describe" => run_describe(args),
        "apply" => run_apply(args),
        _ => passthrough(args),
    }
}

fn passthrough(args: &[String]) -> Result<i32> {
    let mut argv = vec!["kubectl".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig kubectl {}", args.join(" "));
    let out = try_toml_filter(&argv, &run.merged).unwrap_or_else(|| run.merged.clone());
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

fn run_get(args: &[String]) -> Result<i32> {
    let owns = !args.iter().any(|a| a == "-o" || a == "--output");
    let mut argv = vec!["kubectl".to_string()];
    argv.extend(args.iter().cloned());
    if owns {
        argv.push("-o".to_string());
        argv.push("json".to_string());
    }
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig kubectl {}", args.join(" "));
    if !owns {
        emit(&label, &run, &run.merged, ParseOutcome::Passthrough);
        return Ok(run.exit_code);
    }
    match serde_json::from_str::<Value>(run.stdout.trim()) {
        Ok(v) => emit(&label, &run, &render_get(&v), ParseOutcome::Full),
        Err(_) => match try_toml_filter(&argv, &run.merged) {
            Some(f) => emit(&label, &run, &f, ParseOutcome::Passthrough),
            None => emit(&label, &run, &run.merged, ParseOutcome::Passthrough),
        },
    }
    Ok(run.exit_code)
}

fn run_logs(args: &[String]) -> Result<i32> {
    let mut argv = vec!["kubectl".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig kubectl {}", args.join(" "));
    let out = tail_n(&run.stdout, 200);
    emit(&label, &run, &out, ParseOutcome::Partial);
    Ok(run.exit_code)
}

fn run_describe(args: &[String]) -> Result<i32> {
    let mut argv = vec!["kubectl".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig kubectl {}", args.join(" "));
    let out = compact_describe(&run.stdout);
    emit(&label, &run, &out, ParseOutcome::Partial);
    Ok(run.exit_code)
}

fn run_apply(args: &[String]) -> Result<i32> {
    let mut argv = vec!["kubectl".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig kubectl {}", args.join(" "));
    let out = summarise_apply(&run.merged);
    emit(&label, &run, &out, ParseOutcome::Full);
    Ok(run.exit_code)
}

// ---------- Renderers ----------

pub fn render_get(v: &Value) -> String {
    // Support both single-object and list responses.
    let items: Vec<&Value> = if let Some(arr) = v.get("items").and_then(|x| x.as_array()) {
        arr.iter().collect()
    } else {
        vec![v]
    };
    let mut out = String::from("name\tready\tstatus\trestarts\tage\n");
    for it in items {
        let name = it
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        let kind = it.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        // pods have status.containerStatuses with ready/restartCount
        let (ready, status, restarts) = if kind == "Pod" || it.get("status").is_some() {
            let cs = it
                .get("status")
                .and_then(|s| s.get("containerStatuses"))
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let total = cs.len();
            let ready_n = cs
                .iter()
                .filter(|c| c.get("ready").and_then(|r| r.as_bool()) == Some(true))
                .count();
            let restarts: i64 = cs
                .iter()
                .filter_map(|c| c.get("restartCount").and_then(|r| r.as_i64()))
                .sum();
            let phase = it
                .get("status")
                .and_then(|s| s.get("phase"))
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            (
                format!("{}/{}", ready_n, total),
                phase.to_string(),
                restarts.to_string(),
            )
        } else {
            (String::from("-"), String::from("-"), String::from("-"))
        };
        let age = it
            .get("metadata")
            .and_then(|m| m.get("creationTimestamp"))
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            name, ready, status, restarts, age
        ));
    }
    out
}

pub fn tail_n(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    let mut out = String::new();
    if start > 0 {
        out.push_str(&format!("(showing last {} of {} lines)\n", n, lines.len()));
    }
    for l in &lines[start..] {
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Strip the noisy Events section down to the last 5 events.
pub fn compact_describe(s: &str) -> String {
    let mut out = String::new();
    let mut in_events = false;
    let mut event_lines: Vec<&str> = Vec::new();
    for line in s.lines() {
        if line.starts_with("Events:") {
            in_events = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_events {
            event_lines.push(line);
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !event_lines.is_empty() {
        let n = event_lines.len();
        let start = n.saturating_sub(5);
        if start > 0 {
            out.push_str(&format!("  (... {} earlier events)\n", start));
        }
        for l in &event_lines[start..] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

pub fn summarise_apply(s: &str) -> String {
    let mut created = 0;
    let mut configured = 0;
    let mut unchanged = 0;
    for line in s.lines() {
        if line.ends_with(" created") {
            created += 1;
        } else if line.ends_with(" configured") {
            configured += 1;
        } else if line.ends_with(" unchanged") {
            unchanged += 1;
        }
    }
    format!(
        "{} created, {} configured, {} unchanged\n",
        created, configured, unchanged
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_get_pods_list() {
        let v = json!({"items":[{"kind":"Pod","metadata":{"name":"p1","creationTimestamp":"2026"},"status":{"phase":"Running","containerStatuses":[{"ready":true,"restartCount":0}]}}]});
        let out = render_get(&v);
        assert!(out.contains("p1"));
        assert!(out.contains("Running"));
        assert!(out.contains("1/1"));
    }

    #[test]
    fn renders_get_single_object() {
        let v = json!({"kind":"Pod","metadata":{"name":"only","creationTimestamp":"2026"},"status":{"phase":"Pending","containerStatuses":[]}});
        let out = render_get(&v);
        assert!(out.contains("only"));
    }

    #[test]
    fn apply_summary() {
        let s = "deployment.apps/a created\nservice/b unchanged\ndeployment.apps/c configured\n";
        let out = summarise_apply(s);
        assert!(out.contains("1 created"));
        assert!(out.contains("1 configured"));
        assert!(out.contains("1 unchanged"));
    }

    #[test]
    fn describe_truncates_events() {
        let mut s = String::from("Name: x\nKind: Pod\nEvents:\n");
        for i in 0..10 {
            s.push_str(&format!("  e{}\n", i));
        }
        let out = compact_describe(&s);
        assert!(out.contains("Events:"));
        assert!(out.contains("e9"));
        assert!(out.contains("5 earlier events"));
    }

    #[test]
    fn tail_n_keeps_last_lines() {
        let s = "a\nb\nc\nd\ne\n";
        let out = tail_n(s, 2);
        assert!(out.contains("d"));
        assert!(out.contains("e"));
        assert!(!out.contains("\na\n"));
    }
}
