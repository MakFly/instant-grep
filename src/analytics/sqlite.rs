//! SQLite-backed analytics store for `ig`'s tracking history.
//!
//! Dual-write target (PR #1 of the rtk-iso plan). The legacy JSONL writer
//! in `src/tracking.rs` keeps appending; this module records the same
//! events into a `tracking.db` file under the platform data dir so that
//! later PRs can switch the readers (`ig gain`, `ig session`, …) over
//! to SQL queries.

// Some helpers are only consumed from the production binary path; the
// integration tests `#[path]`-include this file and don't touch them.
#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::tracking::TrackEntry;

/// Resolve the platform data directory for ig.
///
/// - macOS: `~/Library/Application Support/ig`
/// - Linux: `~/.local/share/ig` (`$XDG_DATA_HOME/ig` if set)
/// - Windows: `%APPDATA%/ig`
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ig"))
}

/// Path to the SQLite tracking DB.
pub fn db_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tracking.db"))
}

/// Path to the per-process "JSONL has been migrated" marker.
pub fn migrate_marker_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join(".jsonl-migrated"))
}

/// Aggregated summary for a query window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub count: u64,
    pub original_total: u64,
    pub output_total: u64,
    /// `(command, count)`, sorted desc by count.
    pub top_commands: Vec<(String, u64)>,
}

/// Owned connection wrapper.
pub struct TrackingDb {
    conn: Connection,
}

impl TrackingDb {
    /// Open (and create-if-missing) the tracking DB at the platform path.
    pub fn open() -> Result<Self> {
        let path = db_path().context("no data dir available")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating data dir")?;
        }
        Self::open_at(&path)
    }

    /// Open (and create-if-missing) the DB at an explicit path. Useful for tests.
    pub fn open_at(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).context("open sqlite db")?;
        // WAL is robust under multi-process append + crash; NORMAL is fine
        // for our durability needs (telemetry, not user money).
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("set WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("set synchronous")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                command TEXT NOT NULL,
                project_path TEXT,
                original_bytes INTEGER NOT NULL DEFAULT 0,
                output_bytes INTEGER NOT NULL DEFAULT 0,
                exec_time_ms INTEGER NOT NULL DEFAULT 0,
                exit_code INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_commands_ts ON commands(timestamp);
            CREATE INDEX IF NOT EXISTS idx_commands_project ON commands(project_path);
            "#,
        )
        .context("init schema")?;

        Ok(Self { conn })
    }

    /// Insert one tracking event. Idempotent dedup: `(timestamp, command,
    /// original_bytes, output_bytes)` is treated as a natural key — if a
    /// row with the same tuple exists, the new insert is silently dropped.
    pub fn record(&self, entry: &TrackEntry) -> Result<()> {
        let ts = now_secs();
        self.record_with_ts(entry, ts)
    }

    /// Like `record` but with caller-supplied timestamp (used by JSONL migration
    /// to preserve historical ordering).
    pub fn record_with_ts(&self, entry: &TrackEntry, ts: u64) -> Result<()> {
        // Dedup probe.
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM commands WHERE timestamp = ?1 AND command = ?2 \
                 AND original_bytes = ?3 AND output_bytes = ?4 LIMIT 1",
                params![
                    ts as i64,
                    &entry.command,
                    entry.original_bytes as i64,
                    entry.output_bytes as i64,
                ],
                |row| row.get(0),
            )
            .optional()
            .context("dedup probe")?;
        if existing.is_some() {
            return Ok(());
        }

        self.conn
            .execute(
                "INSERT INTO commands (timestamp, command, project_path, \
                 original_bytes, output_bytes, exec_time_ms, exit_code) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    ts as i64,
                    &entry.command,
                    &entry.project,
                    entry.original_bytes as i64,
                    entry.output_bytes as i64,
                    entry.exec_time_ms.unwrap_or(0) as i64,
                    entry.exit_code,
                ],
            )
            .context("insert command")?;
        Ok(())
    }

    /// Drop every row older than `now - days*86400`. Returns the number
    /// of rows deleted. `days == 0` is treated as "delete everything"
    /// (callers use it to fully wipe the store; the strict `<` cutoff
    /// would otherwise leave `now`-stamped rows behind). Wired into
    /// `ig gc` in a follow-up PR.
    #[allow(dead_code)]
    pub fn cleanup_older_than(&self, days: u32) -> Result<usize> {
        let sql = if days == 0 {
            "DELETE FROM commands"
        } else {
            "DELETE FROM commands WHERE timestamp < ?1"
        };
        let n = if days == 0 {
            self.conn.execute(sql, []).context("cleanup all")?
        } else {
            let cutoff = now_secs().saturating_sub(days as u64 * 86_400);
            self.conn
                .execute(sql, params![cutoff as i64])
                .context("cleanup older")?
        };
        Ok(n)
    }

    /// Aggregated summary over a window.
    pub fn get_summary(
        &self,
        since_days: Option<u32>,
        project_filter: Option<&str>,
    ) -> Result<Summary> {
        let cutoff: i64 = match since_days {
            Some(d) => now_secs().saturating_sub(d as u64 * 86_400) as i64,
            None => 0,
        };

        let (count, original_total, output_total): (u64, u64, u64) = self
            .conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(original_bytes), 0), \
                 COALESCE(SUM(output_bytes), 0) FROM commands \
                 WHERE timestamp >= ?1 \
                 AND (?2 IS NULL OR project_path = ?2)",
                params![cutoff, project_filter],
                |row| {
                    let c: i64 = row.get(0)?;
                    let i: i64 = row.get(1)?;
                    let o: i64 = row.get(2)?;
                    Ok((c as u64, i as u64, o as u64))
                },
            )
            .context("summary aggregate")?;

        let mut top_commands = Vec::new();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT command, COUNT(*) as c FROM commands \
                 WHERE timestamp >= ?1 \
                 AND (?2 IS NULL OR project_path = ?2) \
                 GROUP BY command ORDER BY c DESC LIMIT 15",
            )
            .context("prepare top")?;
        let rows = stmt
            .query_map(params![cutoff, project_filter], |row| {
                let cmd: String = row.get(0)?;
                let c: i64 = row.get(1)?;
                Ok((cmd, c as u64))
            })
            .context("query top")?;
        for r in rows {
            top_commands.push(r?);
        }

        Ok(Summary {
            count,
            original_total,
            output_total,
            top_commands,
        })
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(cmd: &str, in_b: u64, out_b: u64) -> TrackEntry {
        TrackEntry {
            command: cmd.into(),
            original_bytes: in_b,
            output_bytes: out_b,
            project: "/test".into(),
            exec_time_ms: None,
            exit_code: None,
        }
    }

    #[test]
    fn open_creates_db() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("t.db");
        let db = TrackingDb::open_at(&p).unwrap();
        let s = db.get_summary(None, None).unwrap();
        assert_eq!(s.count, 0);
    }

    #[test]
    fn record_then_summary() {
        let dir = TempDir::new().unwrap();
        let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();
        db.record(&entry("ig read foo.rs", 1000, 200)).unwrap();
        let s = db.get_summary(None, None).unwrap();
        assert_eq!(s.count, 1);
        assert_eq!(s.original_total, 1000);
        assert_eq!(s.output_total, 200);
    }
}
