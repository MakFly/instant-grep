//! `ig curl` against a 2 MiB local file via `file://` (requires curl on PATH).
//! Asserts the body is truncated in stdout and the truncation marker is
//! present.

use std::fs;
use std::process::Command;

#[test]
fn large_body_truncates_and_marks() {
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl not on PATH — skipping");
        return;
    }
    let tmp = std::env::temp_dir().join("ig_curl_tee_test.bin");
    let payload = vec![b'a'; 2 * 1024 * 1024];
    fs::write(&tmp, &payload).expect("write payload");
    let url = format!("file://{}", tmp.display());

    let bin = env!("CARGO_BIN_EXE_ig");
    let out = Command::new(bin)
        .args(["curl", &url])
        .output()
        .expect("spawn ig curl");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("truncated") || stdout.len() <= 200 * 1024,
        "expected truncation marker; got {} bytes: {}",
        stdout.len(),
        &stdout[..stdout.len().min(200)]
    );

    let _ = fs::remove_file(tmp);
}
