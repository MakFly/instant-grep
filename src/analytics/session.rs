//! `ig session` — Show ig adoption across Claude Code sessions.
//!
//! Walks session files, counts ig vs raw commands,
//! and identifies missed optimization opportunities.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

/// Run the session analytics and print results.
pub fn run_session(since_days: u32) {
    let sessions = find_sessions(since_days);
    if sessions.is_empty() {
        println!("No sessions found in the last {} days.", since_days);
        return;
    }

    let mut total_commands: usize = 0;
    let mut ig_commands: usize = 0;
    let mut missed: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();

    for path in &sessions {
        if let Ok(cmds) = extract_bash_commands(path) {
            for cmd in &cmds {
                total_commands += 1;
                let trimmed = cmd.trim();

                if trimmed.starts_with("ig ") || trimmed == "ig" {
                    ig_commands += 1;
                    continue;
                }

                // Detect commands that could have been ig
                if let Some(replacement) = detect_missed(trimmed) {
                    *missed.entry(replacement).or_insert(0) += 1;
                }
            }
        }
    }

    let raw_commands = total_commands - ig_commands;
    let missed_total: usize = missed.values().sum();
    let eligible = ig_commands + missed_total;
    let adoption_rate = if eligible > 0 {
        (ig_commands as f64 / eligible as f64) * 100.0
    } else {
        100.0
    };

    println!("ig Session Analytics (last {} days)", since_days);
    println!("════════════════════════════════════════════");
    println!("Sessions scanned:     {}", sessions.len());
    println!("Total Bash commands:  {}", format_number(total_commands));
    println!();
    println!(
        "ig commands:          {} ({:.0}%)",
        format_number(ig_commands),
        if total_commands > 0 {
            ig_commands as f64 / total_commands as f64 * 100.0
        } else {
            0.0
        }
    );
    println!(
        "Raw commands:         {} ({:.0}%)",
        format_number(raw_commands),
        if total_commands > 0 {
            raw_commands as f64 / total_commands as f64 * 100.0
        } else {
            0.0
        }
    );
    println!(
        "Adoption rate:        {:.0}% of eligible commands",
        adoption_rate
    );

    if !missed.is_empty() {
        println!();
        println!("Top missed commands:");

        let mut missed_sorted: Vec<_> = missed.into_iter().collect();
        missed_sorted.sort_by(|a, b| b.1.cmp(&a.1));

        for (replacement, count) in missed_sorted.iter().take(10) {
            println!("  {} ({} times)", replacement, count);
        }
    }
    println!();
}

/// Detect if a raw command could have been an ig command.
/// Returns the suggested replacement description.
fn detect_missed(cmd: &str) -> Option<&'static str> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let base = parts.first().copied().unwrap_or("");

    match base {
        "cat" | "head" | "tail" => Some("cat/head/tail → ig read"),
        "grep" | "rg" => Some("grep/rg → ig search"),
        "find" => Some("find → ig files / ig search"),
        "tree" => Some("tree → ig ls"),
        "ls" if parts
            .iter()
            .any(|p| *p == "-la" || *p == "-lah" || *p == "-al") =>
        {
            Some("ls -la → ig ls")
        }
        "wc" => Some("wc → ig read (line count in header)"),
        _ => None,
    }
}

/// Find `.jsonl` session files modified within the time window.
fn find_sessions(since_days: u32) -> Vec<PathBuf> {
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return Vec::new(),
    };

    let projects_dir = home.join(".claude/projects");
    if !projects_dir.is_dir() {
        return Vec::new();
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(since_days) * 86400))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut sessions = Vec::new();
    for entry in walkdir::WalkDir::new(&projects_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(meta) = path.metadata()
            && let Ok(mtime) = meta.modified()
                && mtime >= cutoff {
                    sessions.push(path.to_path_buf());
                }
    }
    sessions
}

// --- JSONL parsing (simplified, reuses same structure as learn.rs) ---

#[derive(Deserialize)]
struct SessionMessage {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ToolUseBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    name: Option<String>,
    input: Option<ToolInput>,
}

#[derive(Deserialize)]
struct ToolInput {
    command: Option<String>,
}

/// Extract all Bash commands from a session file.
fn extract_bash_commands(path: &Path) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let mut commands = Vec::new();

    for line in content.lines() {
        let msg: SessionMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.role.as_deref() != Some("assistant") {
            continue;
        }

        let content_val = match msg.content {
            Some(v) => v,
            None => continue,
        };

        let blocks = match content_val.as_array() {
            Some(arr) => arr,
            None => continue,
        };

        for block in blocks {
            if let Ok(tu) = serde_json::from_value::<ToolUseBlock>(block.clone())
                && tu.block_type.as_deref() == Some("tool_use")
                    && tu.name.as_deref() == Some("Bash")
                    && let Some(input) = tu.input
                        && let Some(cmd) = input.command {
                            commands.push(cmd);
                        }
        }
    }

    Ok(commands)
}

fn format_number(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_missed() {
        assert!(detect_missed("cat src/main.rs").is_some());
        assert!(detect_missed("grep -n pattern file.rs").is_some());
        assert!(detect_missed("rg pattern").is_some());
        assert!(detect_missed("find . -name '*.rs'").is_some());
        assert!(detect_missed("tree").is_some());
        assert!(detect_missed("ls -la").is_some());
        assert!(detect_missed("cargo test").is_none());
        assert!(detect_missed("ig search pattern").is_none());
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(2_340), "2,340");
        assert_eq!(format_number(1_500_000), "1.5M");
    }
}
