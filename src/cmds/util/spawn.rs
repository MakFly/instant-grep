//! Shared spawn helpers used by every per-tool parser.
//!
//! Captures stdout + stderr, returns merged UTF-8 string (lossy), the exit
//! code, and emits the standard "tool not found in PATH" error path so each
//! parser doesn't have to duplicate it.

use std::process::Command;

use anyhow::Result;

/// Outcome of running a wrapped tool.
pub struct CapturedRun {
    pub stdout: String,
    pub stderr: String,
    pub merged: String,
    pub exit_code: i32,
}

/// Spawn `argv[0]` with the remaining args. On `ENOENT` (binary not in
/// PATH), prints `(tool '<bin>' not found in PATH)` and returns `Ok(127)`
/// so the caller can `std::process::exit(127)`.
pub fn capture(argv: &[String]) -> Result<Option<CapturedRun>> {
    if argv.is_empty() {
        anyhow::bail!("empty argv");
    }
    match Command::new(&argv[0]).args(&argv[1..]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let merged = if stderr.is_empty() {
                stdout.clone()
            } else if stdout.is_empty() {
                stderr.clone()
            } else {
                format!("{}{}", stdout, stderr)
            };
            let exit_code = output.status.code().unwrap_or(1);
            Ok(Some(CapturedRun {
                stdout,
                stderr,
                merged,
                exit_code,
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("(tool '{}' not found in PATH)", argv[0]);
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
