//! `ig tree [path]` — wrapper for `tree`. Caps depth at 3 by default and
//! falls back to `ig ls` when `tree` is not installed.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let has_depth = args.iter().any(|a| a == "-L" || a == "--level");
    let mut argv = vec!["tree".to_string()];
    if !has_depth {
        argv.push("-L".to_string());
        argv.push("3".to_string());
    }
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        // tree not installed → defer to ig ls
        eprintln!("(tree not installed; tip: run `ig ls` instead)");
        return Ok(127);
    };
    let label = format!("ig tree {}", args.join(" "));
    let out = try_toml_filter(&argv, &run.merged).unwrap_or_else(|| run.merged.clone());
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles() {
        // Compile-only sanity check — integration coverage in tests/.
        let _f = super::run;
    }
}
