//! PR #1 — round-trip TrackingDb: open temp DB, record entries,
//! query summary, verify cleanup_older_than works.

// Local stub of `crate::tracking` so the `#[path]`-included
// `src/analytics/sqlite.rs` can resolve `crate::tracking::TrackEntry`.
pub mod tracking {
    #[derive(Clone, Debug, Default)]
    pub struct TrackEntry {
        pub command: String,
        pub original_bytes: u64,
        pub output_bytes: u64,
        pub project: String,
        pub exec_time_ms: Option<u64>,
        pub exit_code: Option<i32>,
    }
}

// `src/analytics/sqlite.rs` does `use crate::tracking::TrackEntry;` — works
// because of the module above. It also does `use rusqlite::...` and
// `use anyhow::...` which resolve through the test binary's deps.
#[path = "../src/analytics"]
mod analytics {
    pub mod sqlite;
}

use analytics::sqlite::TrackingDb;
use tracking::TrackEntry;

fn entry(cmd: &str, in_b: u64, out_b: u64) -> TrackEntry {
    TrackEntry {
        command: cmd.into(),
        original_bytes: in_b,
        output_bytes: out_b,
        project: "/test".into(),
        ..Default::default()
    }
}

#[test]
fn round_trip_record_and_summary() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();

    for i in 0..5 {
        db.record(&entry(
            &format!("ig read f{}.rs", i),
            1000 + i * 100,
            200 + i * 10,
        ))
        .unwrap();
    }

    let s = db.get_summary(None, None).unwrap();
    assert_eq!(s.count, 5);
    assert!(s.original_total > 0);
    assert!(s.output_total > 0);
    assert!(!s.top_commands.is_empty());
}

#[test]
fn cleanup_zero_days_removes_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();
    for i in 0..3 {
        db.record(&entry(&format!("c{}", i), 10, 5)).unwrap();
    }
    assert_eq!(db.get_summary(None, None).unwrap().count, 3);
    let removed = db.cleanup_older_than(0).unwrap();
    assert_eq!(removed, 3);
    assert_eq!(db.get_summary(None, None).unwrap().count, 0);
}

#[test]
fn project_filter_isolates_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = TrackingDb::open_at(&dir.path().join("t.db")).unwrap();
    db.record(&TrackEntry {
        command: "x".into(),
        project: "/a".into(),
        original_bytes: 1,
        output_bytes: 1,
        ..Default::default()
    })
    .unwrap();
    db.record(&TrackEntry {
        command: "y".into(),
        project: "/b".into(),
        original_bytes: 1,
        output_bytes: 1,
        ..Default::default()
    })
    .unwrap();
    let s = db.get_summary(None, Some("/a")).unwrap();
    assert_eq!(s.count, 1);
}
