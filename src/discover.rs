//! Discover missed token-saving opportunities by scanning Claude Code session history.
//! Parses JSONL session files, extracts Bash tool calls, and tests them against
//! the rewrite engine to find commands that could have been rewritten.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::rewrite::{RewriteResult, classify_command};
use crate::util::format_bytes;

pub fn run_discover(since_days: u32, limit: usize) {
    let sessions_dir = claude_projects_dir();
    if !sessions_dir.exists() {
        eprintln!("No Claude Code sessions found at {}", sessions_dir.display());
        return;
    }

    let cutoff = if since_days > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(since_days as u64 * 86400)
    } else {
        0
    };

    let mut missed: BTreeMap<String, CmdStats> = BTreeMap::new();
    let mut unhandled: BTreeMap<String, u64> = BTreeMap::new();
    let mut total_commands = 0u64;
    let mut total_rewritable = 0u64;

    // Walk all project directories
    for entry in fs::read_dir(&sessions_dir).into_iter().flatten().flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }

        // Walk JSONL files in each project
        for file_entry in fs::read_dir(&project_dir).into_iter().flatten().flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            // Check file modification time against cutoff
            if cutoff > 0 {
                if let Ok(meta) = fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        let mtime = modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        if mtime < cutoff {
                            continue;
                        }
                    }
                }
            }

            // Parse the JSONL file
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };

            for line in content.lines() {
                // Quick check: skip lines that don't look like they contain Bash tool calls
                if !line.contains("\"Bash\"") || !line.contains("\"tool_use\"") {
                    continue;
                }

                // Extract Bash commands from the line
                for cmd in extract_bash_commands(line) {
                    total_commands += 1;

                    match classify_command(&cmd) {
                        RewriteResult::Rewrite(_) => {
                            // This command COULD be rewritten — it's a missed saving
                            total_rewritable += 1;
                            let key = command_key(&cmd);
                            let stats = missed.entry(key).or_default();
                            stats.count += 1;
                            // Estimate savings based on typical compression ratios
                            stats.estimated_bytes += estimate_output_bytes(&cmd);
                        }
                        RewriteResult::Passthrough => {
                            // Not rewritable — track for "top unhandled" report
                            let key = command_key(&cmd);
                            *unhandled.entry(key).or_insert(0) += 1;
                        }
                        RewriteResult::Deny(_) | RewriteResult::Ask(_) => {
                            // Deny/ask — these are handled correctly, skip
                        }
                    }
                }
            }
        }
    }

    // Display results
    eprintln!("\x1b[1mig discover — Missed Token Savings\x1b[0m");
    eprintln!("════════════════════════════════════════════════════════════");
    eprintln!();
    eprintln!(
        "Scanned: {} Bash commands (last {}d)",
        total_commands, since_days
    );
    eprintln!(
        "Rewritable: {} ({:.1}%)",
        total_rewritable,
        if total_commands > 0 {
            total_rewritable as f64 / total_commands as f64 * 100.0
        } else {
            0.0
        }
    );
    eprintln!();

    if !missed.is_empty() {
        eprintln!("\x1b[33mMISSED SAVINGS\x1b[0m (commands ig can rewrite but weren't)");
        eprintln!("────────────────────────────────────────────────────────────");
        eprintln!(
            "  {:<35} {:>6}  {:>10}",
            "Command Pattern", "Count", "Est. Saved"
        );
        eprintln!("────────────────────────────────────────────────────────────");

        let mut sorted: Vec<_> = missed.into_iter().collect();
        sorted.sort_by(|a, b| b.1.count.cmp(&a.1.count));

        for (cmd, stats) in sorted.iter().take(limit) {
            eprintln!(
                "  {:<35} {:>6}  {:>10}",
                cmd,
                stats.count,
                format_bytes(stats.estimated_bytes)
            );
        }
        eprintln!("────────────────────────────────────────────────────────────");
    } else {
        eprintln!("\x1b[32mNo missed savings — all rewritable commands are being caught!\x1b[0m");
    }

    eprintln!();

    if !unhandled.is_empty() {
        eprintln!("TOP UNHANDLED (frequent commands ig doesn't cover yet)");
        eprintln!("────────────────────────────────────────────────────────────");
        eprintln!("  {:<40} {:>6}", "Command Pattern", "Count");
        eprintln!("────────────────────────────────────────────────────────────");

        let mut sorted: Vec<_> = unhandled.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        for (cmd, count) in sorted.iter().take(limit) {
            eprintln!("  {:<40} {:>6}", cmd, count);
        }
        eprintln!("────────────────────────────────────────────────────────────");
    }
}

/// Extract Bash command strings from a JSONL line.
/// Format: assistant messages contain content[] with tool_use entries where name=="Bash"
fn extract_bash_commands(line: &str) -> Vec<String> {
    let mut commands = Vec::new();

    // Fast path: parse with serde_json
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return commands;
    };

    // Only process assistant messages
    if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return commands;
    }

    // Walk content array
    if let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && item.get("name").and_then(|n| n.as_str()) == Some("Bash")
            {
                if let Some(cmd) = item
                    .get("input")
                    .and_then(|i| i.get("command"))
                    .and_then(|c| c.as_str())
                {
                    // Skip compound commands (ig can't rewrite those anyway)
                    let trimmed = cmd.trim();
                    if !trimmed.is_empty() && trimmed.len() < 500 {
                        commands.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    commands
}

/// Normalize a command to a key for grouping
fn command_key(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return "unknown".to_string();
    }

    match parts[0] {
        "git" => {
            if parts.len() >= 2 {
                format!("git {}", parts[1])
            } else {
                "git".to_string()
            }
        }
        "cat" | "head" | "tail" | "grep" | "egrep" | "fgrep" | "rg" | "find" | "ls" | "tree" => {
            parts[0].to_string()
        }
        _ => {
            if parts.len() >= 2 {
                format!("{} {}", parts[0], parts[1])
            } else {
                parts[0].to_string()
            }
        }
    }
}

/// Rough estimate of bytes that would be saved by rewriting a command
fn estimate_output_bytes(cmd: &str) -> u64 {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return 0;
    }
    // Conservative estimates based on typical output sizes
    match parts[0] {
        "git" if parts.len() >= 2 => match parts[1] {
            "status" => 400,
            "log" => 5000,
            "diff" => 10000,
            "show" => 5000,
            _ => 200,
        },
        "cat" | "head" | "tail" => 2000,
        "grep" | "egrep" | "fgrep" | "rg" => 3000,
        "find" => 500,
        "tree" => 1000,
        "ls" => 300,
        _ => 200,
    }
}

fn claude_projects_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".claude/projects")
}

#[derive(Default)]
struct CmdStats {
    count: u64,
    estimated_bytes: u64,
}
