//! PR #1 — golden JSONL → SQLite migration test.
//!
//! Re-uses the same `#[path]`-stubbed pattern as `tracking_sqlite.rs`
//! since `instant-grep` is a binary-only crate.

pub mod tracking {
    #[derive(Clone, Debug, Default)]
    pub struct TrackEntry {
        pub command: String,
        pub original_bytes: u64,
        pub output_bytes: u64,
        pub project: String,
        pub exec_time_ms: Option<u64>,
        pub exit_code: Option<i32>,
        pub parse_outcome: Option<String>,
    }
}

#[path = "../src/analytics"]
mod analytics {
    pub mod migrate;
    pub mod sqlite;
}

use analytics::migrate::migrate_jsonl_to_sqlite;
use analytics::sqlite::TrackingDb;

const FIXTURE: &str = "tests/fixtures/tracking_history.jsonl";

#[test]
fn migrate_50_entries() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();

    let report =
        migrate_jsonl_to_sqlite(std::path::Path::new(FIXTURE), &db).expect("first migration");
    assert_eq!(report.total_lines, 50);
    assert_eq!(report.inserted, 50);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.malformed, 0);

    let s = db.get_summary(None, None).unwrap();
    assert_eq!(s.count, 50);
    assert!(s.original_total > 0);
    assert!(s.output_total > 0);
}

#[test]
fn migrate_is_idempotent() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();

    let first = migrate_jsonl_to_sqlite(std::path::Path::new(FIXTURE), &db).unwrap();
    assert_eq!(first.inserted, 50);

    let second = migrate_jsonl_to_sqlite(std::path::Path::new(FIXTURE), &db).unwrap();
    assert_eq!(second.inserted, 0, "re-running must insert zero rows");
    assert_eq!(second.skipped, 50);

    let s = db.get_summary(None, None).unwrap();
    assert_eq!(s.count, 50, "row count must be stable");
}

#[test]
fn missing_jsonl_is_ok() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();
    let report = migrate_jsonl_to_sqlite(&dir.path().join("does-not-exist.jsonl"), &db).unwrap();
    assert_eq!(report.total_lines, 0);
    assert_eq!(report.inserted, 0);
}
