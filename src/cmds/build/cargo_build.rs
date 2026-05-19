//! `ig cargo_build [args...]` — wrap `cargo build` and surface only
//! compile errors plus the final `Finished` line.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
    strip_ansi,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["cargo".to_string(), "build".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("cargo") {
        user.remove(0);
    }
    if user.first().map(|s| s.as_str()) == Some("build") {
        user.remove(0);
    }
    argv.extend(user);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig cargo_build {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (filter_cargo(&run.merged), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

fn filter_cargo(raw: &str) -> String {
    let cleaned = strip_ansi(raw);
    let mut keep = Vec::new();
    let mut in_err = false;
    for line in cleaned.lines() {
        let t = line.trim_start();
        if t.starts_with("error[")
            || t.starts_with("error:")
            || t.starts_with("warning:")
            || t.starts_with("warning[")
        {
            in_err = true;
        }
        if in_err {
            keep.push(line.to_string());
            if t.is_empty() {
                in_err = false;
            }
        }
        if (t.starts_with("Finished")
            || t.starts_with("Compiling")
            || t.starts_with("error:")
            || t.starts_with("error["))
            && !keep.last().map(|s| s == line).unwrap_or(false)
        {
            keep.push(line.to_string());
        }
    }
    if keep.is_empty() {
        cleaned.into_owned()
    } else {
        keep.join("\n") + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_finished_line() {
        let s = "    Compiling foo v0.1.0\n    Finished release [optimized] target(s) in 12.3s\n";
        let f = filter_cargo(s);
        assert!(f.contains("Finished"));
    }

    #[test]
    fn keeps_error_block() {
        let s = "    Compiling foo v0.1.0\nerror[E0277]: bad trait\n  --> src/lib.rs:1:1\n\n    Finished\n";
        let f = filter_cargo(s);
        assert!(f.contains("E0277"));
    }
}
