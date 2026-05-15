//! End-to-end regression tests for the v1.19.7 review fixes.
//!
//! Each test exercises the real `ig` binary through subprocess invocation
//! with an isolated XDG cache so it cannot interfere with the user's
//! running daemon. Targets:
//!
//!   1. issue #1 — `ig hold end` blocks until the seal/index is updated.
//!      Regression: `ig hold begin` still succeeds when daemon RSS is above
//!      the soft limit but below the hard limit.
//!   2. issue #2 — daemon and in-process `--type` resolve the same aliases.
//!   3. issue #3 — `ig rewrite` emits shell-safe single-quoted patterns
//!      for inputs containing metacharacters (`$()`, backticks, `;`, `"`).
//!   4. issue #6 — `ig "pat"` does not panic on UTF-8 lines that exceed
//!      the compact-output truncation limit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use tempfile::TempDir;

/// Path of the `ig` binary cargo built for us.
fn ig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ig")
}

/// Build a `Command` for `ig` that is fully isolated from the user's real
/// XDG cache and any daemon they may already have running. Each test owns
/// its own tempdir.
fn ig_cmd(cache_dir: &Path) -> Command {
    let mut cmd = Command::new(ig_bin());
    cmd.env("IG_CACHE_DIR", cache_dir)
        // Defensive: stop the test from accidentally writing into the
        // user's real `~/.claude/`, `~/.codex/`, etc. when `ig setup` paths
        // ever run.
        .env("IG_NO_SETUP_MANAGED_BLOCK", "1")
        .env_remove("XDG_CACHE_HOME")
        // Logs from a failing test go to the captured stderr.
        .stdin(Stdio::null());
    cmd
}

/// Create a minimal project tree under `tmp/project` with a handful of
/// `.rs` and `.ts` files. The content is small enough to index in well
/// under a second.
fn seed_project(tmp: &Path) -> PathBuf {
    let proj = tmp.join("project");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::create_dir_all(proj.join("web")).unwrap();
    fs::write(
        proj.join("Cargo.toml"),
        "[package]\nname=\"p\"\nversion=\"0.0.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        proj.join("src/lib.rs"),
        "pub fn hello() -> &'static str { \"hello world rust\" }\n",
    )
    .unwrap();
    fs::write(
        proj.join("src/main.rs"),
        "fn main() { println!(\"main rust\"); }\n",
    )
    .unwrap();
    fs::write(
        proj.join("web/app.ts"),
        "export const greet = (): string => 'hello world ts';\n",
    )
    .unwrap();
    fs::write(
        proj.join("web/app.tsx"),
        "export const Greet = () => <div>hello tsx</div>;\n",
    )
    .unwrap();
    proj
}

/// Run `ig index <proj>` once to populate the cache so subsequent searches
/// don't take the brute-force fallback path.
fn build_index(cache: &Path, proj: &Path) {
    let out = ig_cmd(cache)
        .arg("index")
        .arg(proj)
        .output()
        .expect("spawn ig index");
    assert!(
        out.status.success(),
        "ig index failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Stop any daemon launched under `cache_dir` so the tempdir can be
/// removed cleanly and the test does not leak a child process.
fn stop_daemon(cache: &Path) {
    let _ = ig_cmd(cache).arg("daemon").arg("stop").output();
}

/// Daemon tests need to be serialised because spawning a daemon under an
/// isolated `IG_CACHE_DIR` is fine, but mixing daemon-vs-in-process runs in
/// parallel can race on the socket. A global mutex keeps things simple.
static DAEMON_TEST_LOCK: Mutex<()> = Mutex::new(());

// ─── Issue #1 ──────────────────────────────────────────────────────────────

#[test]
fn e2e_hold_end_blocks_until_index_visible() {
    let _g = DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let proj = seed_project(tmp.path());
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();

    build_index(&cache, &proj);

    // 1. Acquire the session lock — buffered writes won't trigger overlay
    //    rebuilds until the matching `hold end`.
    let beg = ig_cmd(&cache)
        .arg("hold")
        .arg("begin")
        .arg(&proj)
        .output()
        .unwrap();
    assert!(beg.status.success(), "hold begin failed: {:?}", beg);

    // 2. Inject a brand-new file with a unique token. Without the fix, the
    //    daemon would still be on its old seal at the moment `hold end`
    //    returns and the very next search would miss the token.
    let token = "E2E_HOLD_END_TOKEN_e2c7f12a";
    fs::write(proj.join("src/probe.rs"), format!("// canary {}\n", token)).unwrap();

    // Give the FS watcher (FSEvents on macOS, inotify on Linux) time to
    // observe the new file and emit an event into the daemon's session
    // buffer. Without this, FSEvents latency would make `session_buffer`
    // empty at `hold end` time and the test would no longer exercise the
    // flush path — it would silently fall through the no-op fast path.
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // 3. Closing the session must block until the worker has flushed and
    //    bumped the seal. We then look the token up immediately, with NO
    //    intervening sleep — a stale-index regression would fail here.
    let start = Instant::now();
    let end_out = ig_cmd(&cache)
        .arg("hold")
        .arg("end")
        .arg(&proj)
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(end_out.status.success(), "hold end failed: {:?}", end_out);
    // The blocking call should still return promptly on an empty/small
    // queue. 30 s is the daemon-side timeout; anything close to it would
    // indicate the ack was dropped.
    assert!(
        elapsed.as_secs() < 10,
        "hold end took too long: {:?}",
        elapsed
    );

    let search = ig_cmd(&cache)
        .arg("-c")
        .arg(token)
        .arg(&proj)
        .output()
        .unwrap();
    assert!(search.status.success() || !search.stdout.is_empty());
    let count_output = String::from_utf8_lossy(&search.stdout);
    assert!(
        count_output.contains(token) || count_output.contains(":1"),
        "post-hold-end search did not find the canary token. \
         stdout={:?} stderr={:?}",
        count_output,
        String::from_utf8_lossy(&search.stderr)
    );

    stop_daemon(&cache);
}

#[test]
fn e2e_hold_begin_survives_soft_rss_pressure() {
    let _g = DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let proj = seed_project(tmp.path());
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();

    build_index(&cache, &proj);

    let beg = ig_cmd(&cache)
        .env("IG_DAEMON_SOFT_RSS_MB", "1")
        .env("IG_DAEMON_HARD_RSS_MB", "4096")
        .arg("hold")
        .arg("begin")
        .arg(&proj)
        .output()
        .unwrap();
    assert!(
        beg.status.success(),
        "hold begin failed under soft pressure: stdout={} stderr={}",
        String::from_utf8_lossy(&beg.stdout),
        String::from_utf8_lossy(&beg.stderr)
    );

    let end = ig_cmd(&cache)
        .arg("hold")
        .arg("end")
        .arg(&proj)
        .output()
        .unwrap();
    assert!(
        end.status.success(),
        "hold end failed after soft-pressure begin: stdout={} stderr={}",
        String::from_utf8_lossy(&end.stdout),
        String::from_utf8_lossy(&end.stderr)
    );

    stop_daemon(&cache);
}

// ─── Issue #2 ──────────────────────────────────────────────────────────────

#[test]
fn e2e_daemon_and_inprocess_agree_on_type_alias() {
    let _g = DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let proj = seed_project(tmp.path());
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    build_index(&cache, &proj);

    // The seeded project has app.ts AND app.tsx. `--type ts` must include
    // both (alias resolution) regardless of which search path executes.
    let pattern = "hello";

    // Force the in-process path with IG_NO_DAEMON. The auto-spawn helper
    // also recognises IG_NO_AUTO_DAEMON.
    let inproc = ig_cmd(&cache)
        .env("IG_NO_DAEMON", "1")
        .env("IG_NO_AUTO_DAEMON", "1")
        .arg("-l")
        .arg("-t")
        .arg("ts")
        .arg(pattern)
        .arg(&proj)
        .output()
        .unwrap();
    assert!(inproc.status.success() || !inproc.stdout.is_empty());
    let inproc_files: Vec<&[u8]> = inproc.stdout.split(|b| *b == b'\n').collect();

    let daemon_run = ig_cmd(&cache)
        .arg("-l")
        .arg("-t")
        .arg("ts")
        .arg(pattern)
        .arg(&proj)
        .output()
        .unwrap();
    assert!(daemon_run.status.success() || !daemon_run.stdout.is_empty());
    let daemon_files: Vec<&[u8]> = daemon_run.stdout.split(|b| *b == b'\n').collect();

    let normalize = |mut v: Vec<&[u8]>| {
        v.retain(|line| !line.is_empty());
        v.sort();
        v.len()
    };
    // Don't compare exact bytes (the daemon may print absolute paths and
    // the in-process path relative ones); compare file counts. Both must
    // see app.ts AND app.tsx via the `ts → ts|tsx` alias.
    let inproc_count = normalize(inproc_files);
    let daemon_count = normalize(daemon_files);
    assert!(inproc_count >= 2, "in-process saw {} files", inproc_count);
    assert_eq!(
        inproc_count,
        daemon_count,
        "daemon and in-process disagree on --type ts file count \
         (inproc={}, daemon={}). stdout_inproc={:?} stdout_daemon={:?}",
        inproc_count,
        daemon_count,
        String::from_utf8_lossy(&inproc.stdout),
        String::from_utf8_lossy(&daemon_run.stdout)
    );

    stop_daemon(&cache);
}

// ─── Issue #3 ──────────────────────────────────────────────────────────────

#[test]
fn e2e_rewrite_emits_shell_safe_quoting() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();

    // A pattern packed with shell metacharacters. `ig rewrite` MUST emit
    // something a shell cannot interpret as code: $(...), backticks, ;, ",
    // and \ in a literal pattern should round-trip safely.
    let evil = r#"foo;rm -rf $HOME `echo bad` "x""#;
    let cmd = format!("grep -rn '{}' src/", evil.replace('\'', "'\\''"));
    let out = ig_cmd(&cache).arg("rewrite").arg(&cmd).output().unwrap();
    assert!(out.status.success(), "rewrite failed: {:?}", out);
    let rewritten = String::from_utf8_lossy(&out.stdout).into_owned();

    // We expect the pattern to land in single quotes (the canonical output
    // of our shell_quote helper for anything with metacharacters).
    assert!(
        rewritten.contains('\''),
        "rewrite output is not single-quoted: {:?}",
        rewritten
    );

    // The dangerous fragments must only appear inside single-quoted spans.
    // Strip every '...' span and assert no metacharacter survives in the
    // remaining "shell-active" parts of the rewrite.
    let mut stripped = String::with_capacity(rewritten.len());
    let mut in_quote = false;
    for ch in rewritten.chars() {
        if ch == '\'' {
            in_quote = !in_quote;
            continue;
        }
        if !in_quote {
            stripped.push(ch);
        }
    }
    for bad in [";rm", "$HOME", "`echo", "\""] {
        assert!(
            !stripped.contains(bad),
            "rewrite leaked unquoted shell fragment {:?}: \
             stripped={:?} full={:?}",
            bad,
            stripped,
            rewritten
        );
    }
}

// ─── Issue #6 ──────────────────────────────────────────────────────────────

#[test]
fn e2e_utf8_compact_does_not_panic_on_emoji() {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("project");
    fs::create_dir_all(proj.join("src")).unwrap();
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();

    // Build a single line whose byte length is > MAX_LINE_LEN (120 in src/
    // main.rs) and whose 120th byte falls inside a multi-byte char. We
    // pad with ASCII so we can land the next emoji on byte 119 exactly,
    // then put the match token after it.
    let mut line = String::from("// ");
    for _ in 0..40 {
        line.push_str("abc"); // 120 ascii bytes total → boundary lands at 123
    }
    // Insert a multi-byte char straddling the 120-byte cut point.
    line.replace_range(118..120, "é"); // 'é' = 2 UTF-8 bytes
    line.push_str(" PANIC_PROBE_TOKEN_\u{1f600}\n");

    fs::write(proj.join("src/big.rs"), &line).unwrap();

    // The compact path is what truncates with the old `&s[..MAX_LINE_LEN]`
    // code; force it explicitly.
    let out = ig_cmd(&cache)
        .arg("--compact")
        .arg("PANIC_PROBE_TOKEN_")
        .arg(&proj)
        .output()
        .unwrap();
    // The whole assertion is "no panic" — i.e. the process exited cleanly
    // (status 0 or 1 = no matches, but NOT 101 which is the Rust panic
    // abort code).
    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "ig --compact returned non-clean exit {} (panic?). stderr={:?}",
        code,
        String::from_utf8_lossy(&out.stderr)
    );
}
