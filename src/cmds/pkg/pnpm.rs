//! `ig pnpm [args...]` — wrap pnpm install/add/remove/etc.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["pnpm".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("pnpm") {
        user.remove(0);
    }
    argv.extend(user);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig pnpm {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (super::drop_progress(&run.merged), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}
