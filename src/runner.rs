use std::process::Command;

use anyhow::Result;

use crate::filter::{CompiledFilter, apply_filter};
use crate::tee;
use crate::tracking;

/// Run a command with an optional filter applied to its output.
/// Returns the process exit code.
pub fn run_filtered(args: &[&str], filter: Option<&CompiledFilter>) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("no command provided");
    }

    let output = Command::new(args[0]).args(&args[1..]).output()?;

    let raw = merge_output(&output);
    let exit_code = output.status.code().unwrap_or(1);

    let mut filtered = if let Some(f) = filter {
        apply_filter(f, &raw)
    } else {
        raw.clone()
    };

    // Tee fallback: if the filter hid most of the output on a failing command,
    // save the raw stream and tell the caller where to find it.
    let cmd_str = args.join(" ");
    if tee::should_save(raw.len(), filtered.len(), exit_code)
        && let Some(tee_id) = tee::save(raw.as_bytes(), &cmd_str)
    {
        filtered.push_str(&format!(
            "\n[ig: full output saved — run `ig tee show {}` to read it]\n",
            tee_id
        ));
    }

    print!("{}", filtered);

    // Track token savings
    tracking::log_savings(&tracking::TrackEntry {
        command: format!("ig run {}", cmd_str),
        original_bytes: raw.len() as u64,
        output_bytes: filtered.len() as u64,
        project: std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    });

    Ok(exit_code)
}

/// Run a command as passthrough (no filter) but still track it.
#[allow(dead_code)]
pub fn run_passthrough(args: &[&str]) -> Result<i32> {
    run_filtered(args, None)
}

/// Merge stdout and stderr into a single string.
fn merge_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        stderr.to_string()
    } else {
        format!("{}{}", stdout, stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_output_stdout_only() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"hello\n".to_vec(),
            stderr: vec![],
        };
        assert_eq!(merge_output(&output), "hello\n");
    }

    #[test]
    fn test_merge_output_stderr_only() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: b"error\n".to_vec(),
        };
        assert_eq!(merge_output(&output), "error\n");
    }

    #[test]
    fn test_merge_output_both() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"out\n".to_vec(),
            stderr: b"err\n".to_vec(),
        };
        assert_eq!(merge_output(&output), "out\nerr\n");
    }

    #[test]
    fn test_run_filtered_empty_args() {
        let result = run_filtered(&[], None);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_passthrough_echo() {
        let result = run_passthrough(&["echo", "hello"]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }
}
