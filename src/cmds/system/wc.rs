//! `ig wc [files...]` — thin wrapper for `wc`. Pure passthrough — the raw
//! output is already terse, but we route through `ig` so it lands in
//! `ig gain` tracking.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["wc".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig wc {}", args.join(" "));
    let out = try_toml_filter(&argv, &run.merged).unwrap_or_else(|| run.merged.clone());
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}
