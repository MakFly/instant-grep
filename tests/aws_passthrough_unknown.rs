//! `ig aws <unknown-service> <verb>` must spawn the real `aws` binary and
//! passthrough whatever it prints (or surface the "tool not found" 127 path
//! when `aws` is not on PATH).

use std::process::Command;

#[test]
fn unknown_service_does_not_crash() {
    let bin = env!("CARGO_BIN_EXE_ig");
    let out = Command::new(bin)
        .args(["aws", "totally-unknown-service", "describe-things"])
        .env_remove("PATH")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("spawn ig aws");
    // Either:
    //   - aws is installed → exits with whatever the real aws CLI returned
    //   - aws not installed → exits 127 with `tool 'aws' not found` on stderr
    let code = out.status.code().unwrap_or(0);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if code == 127 {
        assert!(
            stderr.contains("not found"),
            "expected 'not found' on 127: {}",
            stderr
        );
    } else {
        // Any other exit code is fine — we just verify we did not panic.
        assert!(
            !stderr.contains("panicked"),
            "ig must not panic: {}",
            stderr
        );
    }
}
