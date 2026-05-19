//! `ig prisma [args...]` — wrap `prisma` (generate/migrate/db push) and
//! show only success or first error block.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["prisma".to_string()];
    let mut user: Vec<String> = args.to_vec();
    if user.first().map(|s| s.as_str()) == Some("prisma") {
        user.remove(0);
    }
    if user.is_empty() {
        argv.push("generate".to_string());
    }
    argv.extend(user);
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig prisma {}", args.join(" "));
    let (output, outcome) = match try_toml_filter(&argv, &run.merged) {
        Some(f) => (f, ParseOutcome::Passthrough),
        None => (filter_prisma(&run.merged), ParseOutcome::Passthrough),
    };
    emit(&label, &run, &output, outcome);
    Ok(run.exit_code)
}

fn filter_prisma(raw: &str) -> String {
    // Surface the "✔ Generated" lines and any "Error:" block.
    let mut keep = Vec::new();
    let mut in_err = false;
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("Error:") || t.starts_with("error:") {
            in_err = true;
        }
        if in_err
            || t.starts_with('✔')
            || t.starts_with('✓')
            || t.contains("Generated")
            || t.contains("Applied migration")
        {
            keep.push(line);
        }
        if in_err && t.is_empty() {
            in_err = false;
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
    fn keeps_success_line() {
        let s = "noise\n✔ Generated Prisma Client (v5.0.0) to ./node_modules\n";
        assert!(filter_prisma(s).contains("Generated"));
    }
}
