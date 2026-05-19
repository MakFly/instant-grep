//! `ig log [args]` — wrapper for `log show` (macOS) / `journalctl` (Linux).
//! Deduplicates consecutive identical messages and surfaces the tail (last
//! 200 lines) by default.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let bin = if cfg!(target_os = "macos") {
        "log"
    } else {
        "journalctl"
    };
    let mut argv = vec![bin.to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig log {}", args.join(" "));
    let out = if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        condense(&run.stdout)
    };
    emit(&label, &run, &out, ParseOutcome::Partial);
    Ok(run.exit_code)
}

/// Deduplicate consecutive identical message bodies and keep only the last
/// 200 unique lines.
pub fn condense(s: &str) -> String {
    // Each entry stores (display_line, dedup_key, count).
    let mut dedup: Vec<(String, String, usize)> = Vec::new();
    for line in s.lines() {
        let key = strip_timestamp(line);
        if let Some(last) = dedup.last_mut()
            && last.1 == key
        {
            last.2 += 1;
            continue;
        }
        dedup.push((line.to_string(), key, 1));
    }
    let total = dedup.len();
    let start = total.saturating_sub(200);
    let mut out = String::new();
    if start > 0 {
        out.push_str(&format!("(last 200 of {} unique lines)\n", total));
    }
    for (line, _key, count) in &dedup[start..] {
        if *count > 1 {
            out.push_str(&format!("{} (x{})\n", line, count));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Drop leading timestamp/sequence tokens so consecutive identical messages
/// (different timestamps) compare equal. A token "looks like a timestamp"
/// when it is all digits, contains `:` `-` `/` or `T` between digits, or
/// matches `tN` (rtk-style tag).
fn strip_timestamp(s: &str) -> String {
    let mut tokens: Vec<&str> = s.split_whitespace().collect();
    while let Some(first) = tokens.first() {
        if looks_like_ts(first) {
            tokens.remove(0);
        } else {
            break;
        }
    }
    tokens.join(" ")
}

fn looks_like_ts(t: &str) -> bool {
    if t.is_empty() {
        return false;
    }
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    if !has_digit {
        return false;
    }
    t.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | '.' | '/' | 'T' | 'Z' | '+' | 't'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_consecutive() {
        let s = "2026-05-19 t1 hello\n2026-05-19 t2 hello\n2026-05-19 t3 world\n";
        let out = condense(s);
        assert!(out.contains("x2"));
        assert!(out.contains("world"));
    }

    #[test]
    fn preserves_unique_lines() {
        let s = "a\nb\nc\n";
        let out = condense(s);
        assert!(out.contains("a") && out.contains("b") && out.contains("c"));
    }
}
