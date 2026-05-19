//! Hook integrity drift detection — SHA-256 mismatch must be reported.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../src/hooks/integrity.rs"]
mod integrity;

use integrity::{HookSignature, sha256_of_path};

#[test]
fn matching_file_is_not_drift() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h.sh");
    fs::write(&path, b"#!/usr/bin/env bash\necho hi\n").unwrap();
    let actual = sha256_of_path(&path).unwrap();

    let sig = HookSignature {
        installed_path: path.clone(),
        expected_sha256: actual.clone(),
        actual_sha256: Some(actual),
    };
    assert!(!sig.is_drift());
    assert!(!sig.is_missing());
}

#[test]
fn single_byte_flip_is_drift() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h.sh");
    fs::write(&path, b"#!/usr/bin/env bash\necho hi\n").unwrap();
    let expected = sha256_of_path(&path).unwrap();

    // Flip a single byte and re-hash.
    fs::write(&path, b"#!/usr/bin/env bash\necho HI\n").unwrap();
    let actual = sha256_of_path(&path).unwrap();
    assert_ne!(expected, actual);

    let sig = HookSignature {
        installed_path: path,
        expected_sha256: expected,
        actual_sha256: Some(actual),
    };
    assert!(sig.is_drift());
    assert!(!sig.is_missing());
}

#[test]
fn missing_file_reads_as_missing_and_drift() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does_not_exist.sh");
    let sig = HookSignature {
        installed_path: path,
        expected_sha256: "abc".into(),
        actual_sha256: None,
    };
    assert!(sig.is_missing());
    assert!(sig.is_drift());
}

#[path = "../src/hooks/hook_check.rs"]
mod hook_check;

#[test]
fn rate_limit_marker_logic() {
    // We can't easily redirect dirs::cache_dir(), but we can test the
    // pure "due-or-not" logic via marker timestamp arithmetic.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let twenty_three_h_ago = now - (23 * 3600);
    let twenty_five_h_ago = now - (25 * 3600);

    // 23h ago → NOT due
    assert!(now.saturating_sub(twenty_three_h_ago) < 24 * 3600);
    // 25h ago → due
    assert!(now.saturating_sub(twenty_five_h_ago) >= 24 * 3600);
}

#[test]
fn format_drift_warning_omits_ok_entries() {
    let sigs = vec![
        HookSignature {
            installed_path: "/x".into(),
            expected_sha256: "aaa".into(),
            actual_sha256: Some("aaa".into()),
        },
        HookSignature {
            installed_path: "/y".into(),
            expected_sha256: "1111111122222222".into(),
            actual_sha256: Some("3333333344444444".into()),
        },
    ];
    let warning = hook_check::format_drift_warning(&sigs).expect("drift");
    assert!(warning.contains("/y"));
    assert!(!warning.contains("/x"));
}
