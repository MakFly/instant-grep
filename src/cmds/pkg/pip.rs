//! `ig pip [args...]` — wrap pip install/etc.; suppress "using cached" lines.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["pip".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("pip") {
        user.remove(0);
    }
    argv.extend(user);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig pip {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (filter_pip(&run.merged), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

fn filter_pip(raw: &str) -> String {
    let mut keep = Vec::new();
    let mut in_err = false;
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("ERROR") || t.starts_with("Error") {
            in_err = true;
        }
        if (t.starts_with("Using cached")
            || t.starts_with("Collecting ")
            || t.starts_with("Downloading")
            || t.starts_with("  "))
            && !in_err
        {
            continue;
        }
        keep.push(line);
        if in_err && t.is_empty() {
            in_err = false;
        }
    }
    keep.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drops_using_cached() {
        let s =
            "Collecting numpy\n  Using cached numpy-1.0-cp.whl\nSuccessfully installed numpy-1.0\n";
        let out = filter_pip(s);
        assert!(out.contains("Successfully installed"));
        assert!(!out.contains("Using cached"));
    }
}
