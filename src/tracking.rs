//! Token savings tracking — logs command executions to a JSONL history file.
//! Each ig command can log its output size vs estimated original size.
//! `ig gain` reads this file to display a savings dashboard.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::SystemTime;

/// A single tracked command execution.
pub struct TrackEntry {
    pub command: String,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub project: String,
}

/// Get the history file path.
fn history_path() -> Option<PathBuf> {
    let data_dir = if cfg!(target_os = "macos") {
        dirs_next().map(|h| h.join("Library/Application Support/ig"))
    } else {
        dirs_next().map(|h| h.join(".local/share/ig"))
    };
    data_dir.map(|d| d.join("history.jsonl"))
}

fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Log a command execution to the history file.
pub fn log_savings(entry: &TrackEntry) {
    let Some(path) = history_path() else { return };

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let saved = entry.original_bytes.saturating_sub(entry.output_bytes);
    let pct = if entry.original_bytes > 0 {
        (saved as f64 / entry.original_bytes as f64) * 100.0
    } else {
        0.0
    };

    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let line = format!(
        "{{\"ts\":{},\"cmd\":\"{}\",\"in\":{},\"out\":{},\"saved\":{},\"pct\":{:.1},\"project\":\"{}\"}}\n",
        ts,
        entry
            .command
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r"),
        entry.original_bytes,
        entry.output_bytes,
        saved,
        pct,
        entry
            .project
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r"),
    );

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    // Acquire exclusive lock to prevent concurrent write corruption
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_EX);
    }
    let _ = file.write_all(line.as_bytes());
    // Release lock
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}

/// Parsed history entry for aggregation.
#[derive(Debug)]
pub struct HistoryEntry {
    pub command: String,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub saved_bytes: u64,
    pub timestamp: u64,
    #[allow(dead_code)]
    pub project: String,
}

#[derive(serde::Deserialize)]
struct JsonEntry {
    #[serde(default)]
    ts: u64,
    #[serde(default)]
    cmd: String,
    #[serde(rename = "in", default)]
    in_bytes: u64,
    #[serde(rename = "out", default)]
    out_bytes: u64,
    #[serde(default)]
    saved: u64,
    #[serde(default)]
    project: String,
}

/// Read all history entries.
pub fn read_history() -> Vec<HistoryEntry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let entry: JsonEntry = serde_json::from_str(line).ok()?;
            Some(HistoryEntry {
                command: entry.cmd,
                original_bytes: entry.in_bytes,
                output_bytes: entry.out_bytes,
                saved_bytes: entry.saved,
                timestamp: entry.ts,
                project: entry.project,
            })
        })
        .collect()
}

/// Clear history file.
pub fn clear_history() {
    if let Some(path) = history_path() {
        let _ = fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_entry_parsing() {
        let json = r#"{"ts":1234,"cmd":"ig read file.ts","in":5000,"out":2500,"saved":2500,"pct":50.0,"project":"/test"}"#;
        let entry: JsonEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.cmd, "ig read file.ts");
        assert_eq!(entry.in_bytes, 5000);
        assert_eq!(entry.out_bytes, 2500);
        assert_eq!(entry.saved, 2500);
    }

    #[test]
    fn test_json_entry_with_escaped_quotes() {
        let json = r#"{"ts":1234,"cmd":"ig \"pattern\"","in":5000,"out":2500,"saved":2500,"pct":50.0,"project":"/test"}"#;
        let entry: JsonEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.cmd, r#"ig "pattern""#);
    }

    #[test]
    fn test_log_and_read() {
        // Set HOME to temp dir for isolated test
        let dir = tempfile::TempDir::new().unwrap();
        unsafe { std::env::set_var("HOME", dir.path()) };

        log_savings(&TrackEntry {
            command: "ig read file.ts".into(),
            original_bytes: 5000,
            output_bytes: 2000,
            project: "/test".into(),
        });

        let entries = read_history();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "ig read file.ts");
        assert_eq!(entries[0].original_bytes, 5000);
        assert_eq!(entries[0].output_bytes, 2000);
        assert_eq!(entries[0].saved_bytes, 3000);
    }
}
