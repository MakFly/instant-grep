//! One-shot migration: JSONL tracking history → SQLite.
//!
//! Idempotent — the SQLite layer dedups on `(timestamp, command,
//! original_bytes, output_bytes)`, so re-running the migration inserts
//! zero rows the second time.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::sqlite::TrackingDb;

/// Result of a migration pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub total_lines: usize,
    pub inserted: usize,
    pub skipped: usize,
    pub malformed: usize,
}

#[derive(Deserialize)]
struct JsonlEntry {
    #[serde(default)]
    ts: u64,
    #[serde(default)]
    cmd: String,
    #[serde(rename = "in", default)]
    in_bytes: u64,
    #[serde(rename = "out", default)]
    out_bytes: u64,
    #[serde(default)]
    project: String,
}

/// Replay every line of `jsonl_path` into `db`. Missing file → empty report.
pub fn migrate_jsonl_to_sqlite(jsonl_path: &Path, db: &TrackingDb) -> Result<MigrationReport> {
    let mut report = MigrationReport::default();

    let content = match std::fs::read_to_string(jsonl_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e).context("read jsonl"),
    };

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        report.total_lines += 1;
        let parsed: JsonlEntry = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(_) => {
                report.malformed += 1;
                continue;
            }
        };

        let entry = crate::tracking::TrackEntry {
            command: parsed.cmd,
            original_bytes: parsed.in_bytes,
            output_bytes: parsed.out_bytes,
            project: parsed.project,
            exec_time_ms: None,
            exit_code: None,
        };

        // Probe before insert so we can report skipped vs inserted accurately
        // (record_with_ts dedups but doesn't tell us which path it took).
        let before = db.get_summary(None, None)?.count;
        db.record_with_ts(&entry, parsed.ts)?;
        let after = db.get_summary(None, None)?.count;
        if after > before {
            report.inserted += 1;
        } else {
            report.skipped += 1;
        }
    }

    Ok(report)
}
