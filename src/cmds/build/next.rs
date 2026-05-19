//! `ig next [args...]` — wrap `next` (build/dev/start) and surface the
//! route-size summary (`Route (app) / size / first-load JS`) when present.
//!
//! Strategy: spawn the tool, run output through the TOML filter pipeline if
//! a matching filter exists, else passthrough.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["next".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("next") {
        user.remove(0);
    }
    if user.is_empty() {
        argv.push("build".to_string());
    }
    argv.extend(user);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig next {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (filter_next_output(&run.merged), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

/// Surface the Next build summary section (lines starting with `┌`, `├`,
/// `└`, `Route (`, `└ ƒ`, plus error blocks).
fn filter_next_output(raw: &str) -> String {
    let mut keep = Vec::new();
    let mut in_summary = false;
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("Route (") || t.starts_with("Page") || t.starts_with("┌") {
            in_summary = true;
        }
        if in_summary || t.contains("error") || t.contains("Failed") || t.contains("warn ") {
            keep.push(line);
        }
    }
    if keep.is_empty() {
        raw.to_string()
    } else {
        keep.join("\n") + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_route_summary() {
        let s = "garbage\nRoute (app)                 Size  First Load JS\n┌ ○ /                          0 B          80 kB\n├ ○ /about                     1.2 kB        82 kB\nlast noise\n";
        let f = filter_next_output(s);
        assert!(f.contains("Route (app)"));
        assert!(f.contains("/about"));
    }
}
