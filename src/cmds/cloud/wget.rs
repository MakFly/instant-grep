//! `ig wget [args]` — suppress wget progress noise and surface only the
//! final summary line (`saved [N/N]` or `X saved in Ys`).

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["wget".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig wget {}", args.join(" "));
    let out = if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        compact_wget(&run.merged)
    };
    emit(&label, &run, &out, ParseOutcome::Partial);
    Ok(run.exit_code)
}

pub fn compact_wget(s: &str) -> String {
    let mut summary: Option<&str> = None;
    for line in s.lines() {
        let l = line.trim();
        if l.contains("saved [") || l.contains(" saved in ") || l.starts_with("Saving to:") {
            summary = Some(line);
        }
    }
    match summary {
        Some(s) => format!("{}\n", s),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_summary_saved_line() {
        let s = "\
--2026-05-19 11:00:00-- https://example.com/file.bin
Resolving example.com (example.com)...
HTTP request sent, awaiting response... 200 OK
Length: 12345 (12K) [application/octet-stream]
Saving to: 'file.bin'
0%                          ........................
'file.bin' saved [12345/12345]
";
        let out = compact_wget(s);
        assert!(out.contains("'file.bin' saved [12345/12345]"));
        assert!(!out.contains("Resolving"));
    }
}
