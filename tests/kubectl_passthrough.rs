//! `ig kubectl exec ...` and other non-`get/logs/describe/apply` verbs must
//! fall through to passthrough, never crash, and never inject `-o json`.

use std::process::Command;

#[test]
fn exec_verb_does_not_crash() {
    let bin = env!("CARGO_BIN_EXE_ig");
    let out = Command::new(bin)
        .args(["kubectl", "exec", "--", "echo", "hi"])
        .env_remove("PATH")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("spawn ig kubectl");
    let code = out.status.code().unwrap_or(0);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if code == 127 {
        assert!(stderr.contains("not found"));
    } else {
        assert!(!stderr.contains("panicked"));
    }
}
