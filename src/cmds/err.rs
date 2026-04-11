//! `ig err <command...>` — execute a command and show only errors/warnings.
//!
//! Runs the given command, merges stdout and stderr, then filters output
//! to keep only lines containing error indicators (error, warning, fail, etc.).

use std::process::Command;
use anyhow::Result;
use crate::tracking;

/// Run a command and display only error/warning lines.
pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig err <command...>");
    }

    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}{}", stdout, stderr);
    let exit_code = output.status.code().unwrap_or(1);

    let error_re = regex::Regex::new(
        r"(?i)(error|warning|fail|panic|exception|fatal|critical)"
    ).unwrap();

    let filtered: Vec<&str> = raw
        .lines()
        .filter(|l| error_re.is_match(l))
        .collect();

    let output_text = if filtered.is_empty() {
        println!("[ok] No errors");
        "[ok] No errors".to_string()
    } else {
        for line in &filtered {
            println!("{}", line);
        }
        filtered.join("\n")
    };

    // Track savings
    let project = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    tracking::log_savings(&tracking::TrackEntry {
        command: format!("ig err {}", args.join(" ")),
        original_bytes: raw.len() as u64,
        output_bytes: output_text.len() as u64,
        project,
    });

    Ok(exit_code)
}
