//! PR #4 — verify the vitest parser still succeeds when stdout is prefixed
//! by `pnpm exec`'s dotenv banner / script-name lines.

use std::io::Write;
use std::process::{Command, Stdio};

const RAW: &str = "[dotenv@1.5.0] loaded .env (NODE_ENV)\n\
> my-app@0.0.1 test\n\
> vitest --reporter=json\n\
\n\
{\"numTotalTests\":3,\"numPassedTests\":2,\"numFailedTests\":1,\"numPendingTests\":0,\
\"testResults\":[{\"name\":\"tests/a.test.ts\",\"assertionResults\":[\
{\"fullName\":\"adds\",\"status\":\"passed\",\"failureMessages\":[]},\
{\"fullName\":\"subtracts\",\"status\":\"failed\",\"failureMessages\":[\"Error: expected 0 to be 1\"]},\
{\"fullName\":\"divides\",\"status\":\"passed\",\"failureMessages\":[]}]}],\
\"startTime\":1000.0,\"endTime\":1420.0}\n";

#[test]
fn vitest_parses_through_dotenv_prefix() {
    let bin = env!("CARGO_BIN_EXE_ig");
    let mut child = Command::new(bin)
        .args(["__parse", "vitest"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ig __parse");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(RAW.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("2 passed") && stdout.contains("1 failed"),
        "stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("subtracts"),
        "should surface failure name; stdout: {}",
        stdout
    );
}
