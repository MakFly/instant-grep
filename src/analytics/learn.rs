//! `ig learn` — Detect CLI correction patterns from Claude Code session history.
//!
//! Walks `~/.claude/projects/` recursively, finds `.jsonl` session files,
//! and looks for sequences where a Bash command fails then gets corrected.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

/// A detected correction pattern.
pub struct CorrectionPattern {
    pub error_type: String,
    pub count: usize,
    pub example_wrong: String,
    pub example_right: String,
}

/// A parsed Bash command with its outcome.
struct CommandExec {
    command: String,
    is_error: bool,
    error_text: String,
}

/// Run the learn analysis and print results.
pub fn run_learn(since_days: u32, limit: usize) {
    let sessions = find_sessions(since_days);
    if sessions.is_empty() {
        println!("No sessions found in the last {} days.", since_days);
        return;
    }

    let mut all_commands = Vec::new();
    for path in &sessions {
        if let Ok(cmds) = extract_commands(path) {
            all_commands.extend(cmds);
        }
    }

    let patterns = detect_corrections(&all_commands);

    print_patterns(&patterns, limit, since_days, sessions.len());
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

// --- JSONL parsing types ---

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
    id: Option<String>,
    input: Option<ToolInput>,
}

#[derive(Deserialize)]
struct ToolInput {
    command: Option<String>,
}

#[derive(Deserialize)]
struct ToolResultBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    tool_use_id: Option<String>,
    content: Option<serde_json::Value>,
    is_error: Option<bool>,
}

/// Extract Bash command executions from a session file.
fn extract_commands(path: &Path) -> anyhow::Result<Vec<CommandExec>> {
    let content = std::fs::read_to_string(path)?;

    // First pass: collect all Bash tool_use commands with their IDs
    struct PendingCmd {
        id: String,
        command: String,
    }
    let mut pending: Vec<PendingCmd> = Vec::new();
    let mut results: std::collections::HashMap<String, (bool, String)> =
        std::collections::HashMap::new();

    for line in content.lines() {
        let msg: SessionMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let role = match msg.role.as_deref() {
            Some(r) => r,
            None => continue,
        };

        let content_val = match msg.content {
            Some(v) => v,
            None => continue,
        };

        let blocks = match content_val.as_array() {
            Some(arr) => arr.clone(),
            None => continue,
        };

        if role == "assistant" {
            for block in &blocks {
                if let Ok(tu) = serde_json::from_value::<ToolUseBlock>(block.clone())
                    && tu.block_type.as_deref() == Some("tool_use")
                        && tu.name.as_deref() == Some("Bash")
                        && let (Some(id), Some(input)) = (tu.id, tu.input)
                            && let Some(cmd) = input.command {
                                pending.push(PendingCmd { id, command: cmd });
                            }
            }
        } else if role == "user" {
            for block in &blocks {
                if let Ok(tr) = serde_json::from_value::<ToolResultBlock>(block.clone())
                    && tr.block_type.as_deref() == Some("tool_result")
                        && let Some(id) = tr.tool_use_id {
                            let is_err = tr.is_error.unwrap_or(false);
                            let text = match tr.content {
                                Some(serde_json::Value::String(s)) => s,
                                Some(serde_json::Value::Array(arr)) => arr
                                    .iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                _ => String::new(),
                            };
                            // Also detect errors from output content
                            let has_error_text = contains_error_signal(&text);
                            results.insert(id, (is_err || has_error_text, text));
                        }
            }
        }
    }

    // Build command list with outcomes
    let mut execs = Vec::new();
    for p in pending {
        let (is_error, error_text) = results.remove(&p.id).unwrap_or((false, String::new()));
        execs.push(CommandExec {
            command: p.command,
            is_error,
            error_text,
        });
    }

    Ok(execs)
}

/// Check if output text contains error signals.
fn contains_error_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    let signals = [
        "error",
        "not found",
        "no such file",
        "unknown flag",
        "unexpected argument",
        "permission denied",
        "command not found",
        "cannot find",
        "fatal:",
        "failed to",
        "unrecognized option",
    ];
    signals.iter().any(|s| lower.contains(s))
}

/// Classify an error into a category.
fn classify_error(error_text: &str) -> String {
    let lower = error_text.to_lowercase();
    if lower.contains("unknown flag")
        || lower.contains("unexpected argument")
        || lower.contains("unrecognized option")
    {
        "Unknown flag".into()
    } else if lower.contains("command not found") {
        "Command not found".into()
    } else if lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("cannot find")
    {
        "Wrong path".into()
    } else if lower.contains("permission denied") {
        "Permission denied".into()
    } else if lower.contains("syntax error") || lower.contains("parse error") {
        "Syntax error".into()
    } else {
        "Other error".into()
    }
}

/// Detect correction patterns: command N fails, command N+1 succeeds.
fn detect_corrections(commands: &[CommandExec]) -> Vec<CorrectionPattern> {
    let mut pattern_map: std::collections::HashMap<String, (usize, String, String)> =
        std::collections::HashMap::new();

    for window in commands.windows(2) {
        let (failed, succeeded) = (&window[0], &window[1]);
        if !failed.is_error || succeeded.is_error {
            continue;
        }

        // Both should be non-empty
        if failed.command.is_empty() || succeeded.command.is_empty() {
            continue;
        }

        let category = classify_error(&failed.error_text);
        let entry = pattern_map.entry(category.clone()).or_insert((
            0,
            failed.command.clone(),
            succeeded.command.clone(),
        ));
        entry.0 += 1;
    }

    let mut patterns: Vec<CorrectionPattern> = pattern_map
        .into_iter()
        .map(|(error_type, (count, wrong, right))| CorrectionPattern {
            error_type,
            count,
            example_wrong: truncate_str(&wrong, 40),
            example_right: truncate_str(&right, 40),
        })
        .collect();

    patterns.sort_by(|a, b| b.count.cmp(&a.count));
    patterns
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn print_patterns(patterns: &[CorrectionPattern], limit: usize, since_days: u32, sessions: usize) {
    println!("ig Learn — CLI Correction Patterns");
    println!("════════════════════════════════════════════");

    if patterns.is_empty() {
        println!(
            "No correction patterns found (last {} days, {} sessions).",
            since_days, sessions
        );
        return;
    }

    let shown = patterns.len().min(limit);
    println!(
        "Found {} correction pattern{} (last {} days, {} sessions)\n",
        patterns.len(),
        if patterns.len() == 1 { "" } else { "s" },
        since_days,
        sessions
    );

    println!("  {:<30}  {:>5}  Example", "Pattern", "Count");
    println!("  {}", "─".repeat(70));

    for p in patterns.iter().take(shown) {
        println!(
            "  {:<30}  {:>5}  {} → {}",
            p.error_type, p.count, p.example_wrong, p.example_right
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_error_signal() {
        assert!(contains_error_signal("error: something failed"));
        assert!(contains_error_signal("No such file or directory"));
        assert!(contains_error_signal("bash: foo: command not found"));
        assert!(!contains_error_signal("Success: all tests passed"));
    }

    #[test]
    fn test_classify_error() {
        assert_eq!(classify_error("unknown flag --foo"), "Unknown flag");
        assert_eq!(classify_error("No such file or directory"), "Wrong path");
        assert_eq!(
            classify_error("bash: pytest: command not found"),
            "Command not found"
        );
        assert_eq!(classify_error("something went wrong"), "Other error");
    }

    #[test]
    fn test_detect_corrections() {
        let cmds = vec![
            CommandExec {
                command: "cargo test --nocapture".into(),
                is_error: true,
                error_text: "error: unexpected argument '--nocapture'".into(),
            },
            CommandExec {
                command: "cargo test -- --nocapture".into(),
                is_error: false,
                error_text: String::new(),
            },
            CommandExec {
                command: "cat src/main.ts".into(),
                is_error: true,
                error_text: "No such file or directory".into(),
            },
            CommandExec {
                command: "cat src/main.tsx".into(),
                is_error: false,
                error_text: String::new(),
            },
        ];

        let patterns = detect_corrections(&cmds);
        assert_eq!(patterns.len(), 2);

        let total: usize = patterns.iter().map(|p| p.count).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        assert_eq!(truncate_str("a long string here", 10), "a long st…");
    }
}
