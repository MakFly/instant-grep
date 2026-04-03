//! Discover missed token-saving opportunities by scanning Claude Code session history.
//! Parses JSONL session files, extracts Bash tool calls, and tests them against
//! the rewrite engine to find commands that could have been rewritten.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::rewrite::{RewriteResult, classify_command};
use crate::util::format_bytes;

/// Broader classification for discover — counts commands ig *could* handle,
/// even if the rewrite engine is too conservative to auto-rewrite them.
/// This gives more accurate "missed savings" numbers.
fn discover_classify(cmd: &str) -> RewriteResult {
    let result = classify_command(cmd);
    if matches!(result, RewriteResult::Passthrough) && is_discoverable(cmd) {
        return RewriteResult::Rewrite(String::new());
    }
    result
}

/// Split a compound command on shell operators (with surrounding spaces) to avoid
/// breaking regex patterns that contain `|`.
fn split_first_command(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    for sep in [" | ", " && ", " || ", "; "] {
        if let Some(idx) = trimmed.find(sep) {
            return trimmed[..idx].trim();
        }
    }
    trimmed
}

/// Check if a Passthrough command is something ig could realistically replace.
/// More permissive than the rewrite engine — used only for discover reporting.
fn is_discoverable(cmd: &str) -> bool {
    let trimmed = cmd.trim();

    // For compound commands, check if the first command in the pipeline is discoverable
    let first_cmd = split_first_command(trimmed);

    let parts: Vec<&str> = first_cmd.split_whitespace().collect();
    if parts.is_empty() {
        return false;
    }

    match parts[0] {
        // grep/egrep/fgrep without -r: ig can search single files too
        "grep" | "egrep" | "fgrep" => {
            // Any grep with a pattern arg is replaceable by ig
            parts.iter().skip(1).any(|p| !p.starts_with('-'))
        }
        // cat with flags or multiple files: ig read handles these
        "cat" => parts.len() >= 2,
        // head/tail with complex args: ig read handles these
        "head" | "tail" => parts.len() >= 2,
        // find without -name: ig files can list project files
        "find" => {
            // Only if not destructive (-exec, -delete)
            !parts.iter().any(|p| *p == "-exec" || *p == "-delete")
        }
        // rg is always rewritable (already handled by classify_command,
        // but catches piped versions like `rg pattern | head`)
        "rg" => true,
        // ls with multiple paths
        "ls" => true,
        // wc -l (line counting) — ig can do this
        "wc" => true,
        _ => false,
    }
}

/// Extract the base command name for grouping in discover, including from compound commands.
fn discover_command_key(cmd: &str) -> String {
    let trimmed = cmd.trim();

    // For compound commands, extract the first command
    let first_cmd = split_first_command(trimmed);

    command_key(first_cmd)
}

pub fn run_discover(since_days: u32, limit: usize) {
    let sessions_dir = claude_projects_dir();
    if !sessions_dir.exists() {
        eprintln!(
            "No Claude Code sessions found at {}",
            sessions_dir.display()
        );
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

        // Walk JSONL files recursively (includes subagents/ subdirectories)
        for path in walk_jsonl_files(&project_dir) {
            // Check file modification time against cutoff (generous window to avoid false negatives)
            if cutoff > 0
                && let Ok(meta) = fs::metadata(&path)
                && let Ok(modified) = meta.modified()
            {
                let mtime = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let generous_cutoff = cutoff.saturating_sub(7 * 86400);
                if mtime < generous_cutoff {
                    continue;
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
                for (cmd, line_ts) in extract_bash_commands(line) {
                    // Filter by line timestamp if available
                    if cutoff > 0 && line_ts > 0 && line_ts < cutoff {
                        continue;
                    }

                    total_commands += 1;

                    match discover_classify(&cmd) {
                        RewriteResult::Rewrite(_) => {
                            // This command COULD be rewritten — it's a missed saving
                            total_rewritable += 1;
                            let key = discover_command_key(&cmd);
                            let stats = missed.entry(key).or_default();
                            stats.count += 1;
                            // Estimate savings based on typical compression ratios
                            stats.estimated_bytes += estimate_output_bytes(&cmd);
                        }
                        RewriteResult::Passthrough => {
                            // Not rewritable — track for "top unhandled" report
                            let key = discover_command_key(&cmd);
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

/// Recursively collect all `.jsonl` files under `dir`, up to depth 3.
fn walk_jsonl_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    walk_jsonl_recursive(dir, &mut files, 0);
    files
}

fn walk_jsonl_recursive(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 3 {
        return;
    } // Safety limit
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_jsonl_recursive(&path, files, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

/// Extract Bash command strings from a JSONL line.
/// Returns a Vec of (command, unix_timestamp) pairs.
/// Format: assistant messages contain content[] with tool_use entries where name=="Bash".
/// Also handles progress messages for subagent tool calls.
fn extract_bash_commands(line: &str) -> Vec<(String, u64)> {
    let mut commands = Vec::new();

    // Fast path: parse with serde_json
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return commands;
    };

    let msg_type = value.get("type").and_then(|t| t.as_str());
    if msg_type != Some("assistant") && msg_type != Some("progress") {
        return commands;
    }

    // Extract line-level timestamp (ISO 8601 or unix u64)
    let line_ts = value.get("timestamp").and_then(|t| t.as_u64()).unwrap_or(0);

    // Helper closure: push a command with the line timestamp
    let mut push_cmd = |cmd: &str| {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() && trimmed.len() < 500 {
            commands.push((trimmed.to_string(), line_ts));
        }
    };

    // Walk content array for assistant messages
    if msg_type == Some("assistant")
        && let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
    {
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && item.get("name").and_then(|n| n.as_str()) == Some("Bash")
                && let Some(cmd) = item
                    .get("input")
                    .and_then(|i| i.get("command"))
                    .and_then(|c| c.as_str())
            {
                push_cmd(cmd);
            }
        }
    }

    // Also check progress messages (subagent tool calls)
    if msg_type == Some("progress")
        && let Some(content) = value
            .pointer("/data/message/message/content")
            .and_then(|c| c.as_array())
    {
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && item.get("name").and_then(|n| n.as_str()) == Some("Bash")
                && let Some(cmd) = item
                    .get("input")
                    .and_then(|i| i.get("command"))
                    .and_then(|c| c.as_str())
            {
                push_cmd(cmd);
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
        "wc" => 100,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::RewriteResult;

    // --- discover_classify: broader than classify_command ---

    #[test]
    fn test_discover_grep_without_recursive() {
        // grep without -r is Passthrough in rewrite, but discoverable
        assert!(matches!(
            classify_command("grep pattern file.txt"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("grep pattern file.txt"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_grep_with_recursive_stays_rewrite() {
        // grep with -r is already Rewrite — discover should keep it
        assert!(matches!(
            discover_classify("grep -rn pattern src/"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_cat_with_flags() {
        // cat -n file is Passthrough in rewrite, but discoverable
        assert!(matches!(
            classify_command("cat -n file.txt"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("cat -n file.txt"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_cat_multiple_files() {
        assert!(matches!(
            classify_command("cat a.txt b.txt"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("cat a.txt b.txt"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_find_without_name() {
        // find . -type f is Passthrough in rewrite, but discoverable
        assert!(matches!(
            classify_command("find . -type f"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("find . -type f"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_find_with_exec_stays_passthrough() {
        // find with -exec is NOT discoverable (destructive)
        assert!(matches!(
            discover_classify("find . -exec rm {} ;"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_discover_piped_grep() {
        // grep pattern file | head -5 — compound, but base is grep
        assert!(matches!(
            classify_command("grep pattern file | head -5"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("grep pattern file | head -5"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_piped_cat() {
        assert!(matches!(
            discover_classify("cat file.txt | grep pattern"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_piped_rg() {
        assert!(matches!(
            discover_classify("rg pattern | head -20"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_wc() {
        assert!(matches!(
            discover_classify("wc -l src/*.rs"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_discover_non_rewritable_stays_passthrough() {
        // Commands ig can't replace at all
        assert!(matches!(
            discover_classify("docker exec -it app bash"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("curl -s https://api.example.com"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            discover_classify("ssh -p 22 server"),
            RewriteResult::Passthrough
        ));
    }

    // --- discover_command_key: handles compound commands ---

    #[test]
    fn test_discover_key_piped_command() {
        assert_eq!(discover_command_key("grep pattern file | head -5"), "grep");
        assert_eq!(discover_command_key("cat file.txt | wc -l"), "cat");
    }

    #[test]
    fn test_discover_key_chained_command() {
        assert_eq!(discover_command_key("ls && echo done"), "ls");
    }

    #[test]
    fn test_discover_key_simple_command() {
        assert_eq!(discover_command_key("grep -rn pattern src/"), "grep");
        assert_eq!(discover_command_key("git log --oneline"), "git log");
    }
}
