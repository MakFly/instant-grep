//! PR #4 — verify `go test -json` NDJSON parsing with interleaved
//! package events.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_ig");
    let mut child = Command::new(bin)
        .args(["__parse", "go_test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn interleaved_package_events() {
    let s = r#"
{"Action":"run","Package":"pkg/a","Test":"TestX"}
{"Action":"run","Package":"pkg/b","Test":"TestY"}
{"Action":"output","Package":"pkg/a","Test":"TestX","Output":"--- PASS: TestX (0.01s)\n"}
{"Action":"output","Package":"pkg/b","Test":"TestY","Output":"--- FAIL: TestY (0.02s)\n"}
{"Action":"pass","Package":"pkg/a","Test":"TestX","Elapsed":0.01}
{"Action":"output","Package":"pkg/b","Test":"TestY","Output":"b_test.go:5: oops\n"}
{"Action":"fail","Package":"pkg/b","Test":"TestY","Elapsed":0.02}
{"Action":"pass","Package":"pkg/a","Elapsed":0.05}
{"Action":"fail","Package":"pkg/b","Elapsed":0.05}
"#;
    let out = run(s);
    assert!(out.contains("1 passed"), "got: {}", out);
    assert!(out.contains("1 failed"), "got: {}", out);
    assert!(
        out.contains("pkg/b") && out.contains("TestY"),
        "got: {}",
        out
    );
}
