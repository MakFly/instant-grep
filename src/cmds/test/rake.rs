//! `ig rake [args...]` — thin wrapper that detects whether the project's
//! `rake test` task drives rspec or minitest and delegates to the right
//! parser, falling back to the TOML pipeline otherwise.

use anyhow::Result;
use std::path::Path;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

fn project_uses_rspec(cwd: &Path) -> bool {
    let gemfile = cwd.join("Gemfile");
    let Ok(content) = std::fs::read_to_string(&gemfile) else {
        return false;
    };
    content.contains("rspec")
}

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    let cwd = std::env::current_dir().unwrap_or_default();
    // If the user explicitly called rspec via rake, just delegate.
    if project_uses_rspec(&cwd)
        && args
            .iter()
            .any(|a| a == "spec" || a == "rspec" || a.starts_with("spec:"))
    {
        return super::rspec::run(args, opts);
    }

    // Otherwise: passthrough through TOML filter pipeline.
    let mut argv = vec!["rake".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig rake {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (run.merged.clone(), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}
