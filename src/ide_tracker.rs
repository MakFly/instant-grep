//! Tracks which projects the user is actively working on by reading
//! Claude Code's on-disk state (`~/.claude/projects/<encoded>/<sessionId>.jsonl`)
//! and emitting [`IdeSignal`]s the daemon uses to warm the right tenants
//! proactively.
//!
//! v1 only supports Claude Code (`IdeSource::ClaudeCode`). Cursor and VS Code
//! sources are reserved for v2 — see `docs/specs/SPEC-ide-tracker.md`.
//!
//! Design constraints:
//! - **Read-only**: never mutates Claude's state. Open files normally.
//! - **Cheap polling**: enumerate `~/.claude/projects/`, filter by mtime,
//!   tail the latest JSONL for `Read` tool_use events. ~5 ms per cycle on
//!   a machine with ~50 projects.
//! - **Self-disabling**: if `~/.claude/projects/` doesn't exist, the thread
//!   exits cleanly after the first probe — no spam in `daemon.log`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IdeSource {
    ClaudeCode,
    // Reserved for v2:
    // Cursor,
    // VSCode,
}

impl IdeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            IdeSource::ClaudeCode => "ide-claude",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdeSignal {
    /// Canonical project root the user is working in.
    pub root: PathBuf,
    /// Most recently touched file paths inside `root`, capped at 20.
    pub hot_files: Vec<PathBuf>,
    pub source: IdeSource,
    /// Mtime of the source state file when we observed it. Read by tests
    /// and reserved for future "freshness" heuristics (e.g. ignore signals
    /// older than X). The daemon path doesn't consume it directly today.
    #[allow(dead_code)]
    pub last_seen: SystemTime,
}

/// Default poll cadence. Lower bound is 1 s (clamp in `spawn_tracker`).
const DEFAULT_POLL: Duration = Duration::from_secs(10);

/// Projects whose JSONL last-modified time is older than this are skipped.
/// Default: 5 min — matches what feels "active" in a coding session.
const ACTIVE_WINDOW: Duration = Duration::from_secs(300);

/// Per-signal hot-file cap. Keeps the channel small and the pre-mmap cost
/// bounded regardless of how chatty the Claude session is.
const HOT_FILES_CAP: usize = 20;

/// How many trailing bytes of the JSONL we scan. JSONL lines are typically
/// 1-4 KiB; 64 KiB covers ~16-64 recent tool_use entries which is enough to
/// recover the last 20 unique Read paths.
const JSONL_TAIL_BYTES: u64 = 64 * 1024;

/// Spawn the Claude Code tracker thread. Returns a receiver the daemon's
/// main loop drains. If `~/.claude/projects/` doesn't exist, the thread
/// exits cleanly and the receiver simply never emits.
pub fn spawn_tracker(poll_interval: Duration) -> Receiver<IdeSignal> {
    let (tx, rx) = mpsc::channel();
    let interval = poll_interval.max(Duration::from_secs(1));
    thread::Builder::new()
        .name("ig-ide-tracker".into())
        .spawn(move || run_claude_loop(tx, interval))
        .expect("spawn ig-ide-tracker thread");
    rx
}

fn claude_projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

fn run_claude_loop(tx: Sender<IdeSignal>, interval: Duration) {
    let Some(projects_dir) = claude_projects_dir() else {
        return;
    };
    // Per-root dedup cache: keep the last hot-files fingerprint so we only
    // emit when something actually changed. Bounded implicitly by the LRU on
    // the daemon side; tracker is the producer, daemon is the policy.
    let mut last_emitted: std::collections::HashMap<PathBuf, u64> =
        std::collections::HashMap::new();

    loop {
        if !projects_dir.exists() {
            // Not running Claude Code on this machine, idle without spamming.
            thread::sleep(interval.max(Duration::from_secs(30)));
            continue;
        }

        match scan_claude_projects(&projects_dir) {
            Ok(signals) => {
                for sig in signals {
                    let fingerprint = fingerprint_signal(&sig);
                    if last_emitted.get(&sig.root) == Some(&fingerprint) {
                        continue;
                    }
                    let root_for_cache = sig.root.clone();
                    if tx.send(sig).is_err() {
                        // Daemon dropped the receiver — exit cleanly.
                        return;
                    }
                    last_emitted.insert(root_for_cache, fingerprint);
                }
            }
            Err(_) => {
                // Transient read errors shouldn't kill the tracker; just retry
                // on the next tick.
            }
        }

        thread::sleep(interval);
    }
}

fn fingerprint_signal(sig: &IdeSignal) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for f in &sig.hot_files {
        f.hash(&mut h);
    }
    h.finish()
}

/// Top-level scan: enumerate Claude project dirs, filter by mtime, build a
/// signal per active project.
fn scan_claude_projects(projects_dir: &Path) -> std::io::Result<Vec<IdeSignal>> {
    let cutoff = SystemTime::now()
        .checked_sub(ACTIVE_WINDOW)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut signals = Vec::new();
    for entry in fs::read_dir(projects_dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let dir = entry.path();

        // Pick the newest *.jsonl under this project dir; that's the active
        // session. Skip the project if its newest file is older than ACTIVE_WINDOW.
        let Some((jsonl, mtime)) = newest_jsonl(&dir) else {
            continue;
        };
        if mtime < cutoff {
            continue;
        }

        // Source of truth = the `cwd` field embedded in each JSONL entry.
        // The dir-name decoding is lossy (Claude doesn't escape `-`) so a
        // project like `kweli-project` would decode to `/.../kweli/project`.
        // The JSONL's `cwd` is canonical and unambiguous, so we use it
        // directly and fall back to the lossy dir-name only when the file
        // can't be read.
        let parsed = parse_session_jsonl(&jsonl);

        let root_str = parsed.cwd.or_else(|| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .and_then(decode_claude_project_dir)
                .map(|p| p.to_string_lossy().to_string())
        });
        let Some(root_str) = root_str else { continue };
        let root = PathBuf::from(root_str);

        // Resolve the root to its canonical form. If it doesn't exist on disk
        // anymore (deleted, renamed) skip — no point warming a phantom.
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };

        // Skip non-project roots (most commonly `~` itself, which Claude
        // records when the user runs `claude` from `$HOME`). The daemon's
        // `guard_suspicious_root` would refuse anyway; filtering here keeps
        // `daemon.log` quiet and avoids the dedup defeat.
        if !looks_like_project_root(&canonical_root) {
            continue;
        }

        let hot_files: Vec<PathBuf> = parsed
            .hot_files
            .into_iter()
            .filter(|p| p.starts_with(&canonical_root))
            .collect();

        signals.push(IdeSignal {
            root: canonical_root,
            hot_files,
            source: IdeSource::ClaudeCode,
            last_seen: mtime,
        });
    }
    Ok(signals)
}

/// Return `(path, mtime)` of the most recently modified `*.jsonl` under `dir`,
/// or `None` if none exists.
fn newest_jsonl(dir: &Path) -> Option<(PathBuf, SystemTime)> {
    let mut newest: Option<(PathBuf, SystemTime)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        match &newest {
            Some((_, prev)) if *prev >= mtime => {}
            _ => newest = Some((path, mtime)),
        }
    }
    newest
}

/// Decode Claude Code's project-dir name back to an absolute path.
///
/// Claude stores `<cwd>` with `/` replaced by `-` and a leading `-`, e.g.
/// `/Users/kev/Documents/foo` → `-Users-kev-Documents-foo`.
///
/// Returns `None` when the name doesn't look like an encoded absolute path.
pub fn decode_claude_project_dir(name: &str) -> Option<PathBuf> {
    let stripped = name.strip_prefix('-')?;
    if stripped.is_empty() {
        return None;
    }
    // Claude doesn't escape '-' that appear inside filenames; that ambiguity
    // is by design (collisions are rare and resolved by canonicalize). We
    // mirror their convention.
    let mut path = String::with_capacity(stripped.len() + 1);
    path.push('/');
    path.push_str(&stripped.replace('-', "/"));
    Some(PathBuf::from(path))
}

/// Inverse of [`decode_claude_project_dir`]. Useful for tests; the daemon
/// itself only ever decodes.
#[cfg(test)]
pub fn encode_claude_project_dir(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let trimmed = s.strip_prefix('/')?;
    Some(format!("-{}", trimmed.replace('/', "-")))
}

/// Aggregate parsed from a single session JSONL.
#[derive(Default, Debug)]
struct SessionData {
    /// The `cwd` field of the most recent entry that has one. This is the
    /// canonical project root for the session.
    cwd: Option<String>,
    /// `Read` tool_use file_paths, in encounter order, dedup'd, not yet
    /// filtered by root. The caller drops out-of-root entries.
    hot_files: Vec<PathBuf>,
}

/// Parse the trailing `JSONL_TAIL_BYTES` of a session JSONL. Extracts
/// the latest `cwd` plus the `Read` tool_use file paths.
///
/// Note: the JSONL schema is permissive — sessions started with
/// `claude -p` only emit a couple of entries; older formats put `Read`
/// inside `message.content[]` with `type=tool_use`; newer ones nest the
/// same shape one level deeper. We probe both shapes and tolerate failures
/// silently.
fn parse_session_jsonl(jsonl: &Path) -> SessionData {
    let mut out = SessionData::default();
    let file = match fs::File::open(jsonl) {
        Ok(f) => f,
        Err(_) => return out,
    };
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(_) => return out,
    };
    let size = meta.len();
    let start = size.saturating_sub(JSONL_TAIL_BYTES);

    use std::io::{Seek, SeekFrom};
    let mut file = file;
    if file.seek(SeekFrom::Start(start)).is_err() {
        return out;
    }
    let mut reader = BufReader::new(file);
    if start > 0 {
        let mut discard = Vec::new();
        let _ = reader.read_until(b'\n', &mut discard);
    }

    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        // `cwd` is a flat top-level field on most entry types — capture
        // the latest non-empty one we see (sessions can drift cwd across
        // entries but it's rare; "latest wins" is the right policy).
        if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str())
            && !cwd.is_empty()
        {
            out.cwd = Some(cwd.to_string());
        }

        // Read events. We accept any shape that looks like
        // `*.tool_use.name == "Read"` + `*.tool_use.input.file_path`,
        // scanning the JSON tree shallowly so we cover the two schemas
        // Claude has used historically.
        collect_read_paths(&val, &mut out.hot_files, &mut seen);
    }

    if out.hot_files.len() > HOT_FILES_CAP {
        let drop = out.hot_files.len() - HOT_FILES_CAP;
        out.hot_files.drain(..drop);
    }
    out
}

/// Walk a JSON value looking for objects shaped like
/// `{type:"tool_use", name:"Read", input:{file_path:"…"}}` (in either of
/// Claude's two historical schemas) and push the `file_path` into `ordered`
/// if not already in `seen`.
fn collect_read_paths(
    val: &serde_json::Value,
    ordered: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    match val {
        serde_json::Value::Object(map) => {
            // Detect the leaf shape inline.
            if map.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                && map.get("name").and_then(|v| v.as_str()) == Some("Read")
                && let Some(file_path) = map
                    .get("input")
                    .and_then(|i| i.get("file_path"))
                    .and_then(|v| v.as_str())
            {
                let p = PathBuf::from(file_path);
                if seen.insert(p.clone()) {
                    ordered.push(p);
                }
            }
            for v in map.values() {
                collect_read_paths(v, ordered, seen);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_read_paths(v, ordered, seen);
            }
        }
        _ => {}
    }
}

/// A path counts as a project root when it has a `.git/` directory or any of
/// the standard project markers (Cargo.toml, package.json, …). Also reject
/// `~` and its immediate parent so we don't warm the whole home tree.
fn looks_like_project_root(p: &Path) -> bool {
    if let Some(home) = dirs::home_dir()
        && (p == home || p.parent() == Some(home.as_path()) && p == home.as_path())
    {
        return false;
    }
    if p == Path::new("/") || p == Path::new("/Users") || p == Path::new("/home") {
        return false;
    }
    if p.join(".git").exists() {
        return true;
    }
    const MARKERS: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "composer.json",
        "Gemfile",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "mix.exs",
    ];
    MARKERS.iter().any(|m| p.join(m).exists())
}

pub fn default_poll_interval() -> Duration {
    std::env::var("IG_IDE_TRACKER_POLL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_POLL)
}

pub fn tracker_enabled() -> bool {
    !matches!(
        std::env::var("IG_IDE_TRACKER_ENABLED").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decode_round_trip_for_paths_without_dashes() {
        // Round-trip only works when path segments contain no `-`. Claude
        // Code's encoding intentionally doesn't escape `-`, so paths with
        // dashes are 1-way (decode is best-effort). We don't try to be
        // smarter than Claude.
        let cases = [
            "/Users/kev/Documents/foo",
            "/tmp/instantgrep",
            "/var/folders/h2/abc/T/x",
        ];
        for path_str in cases {
            let p = PathBuf::from(path_str);
            let enc = encode_claude_project_dir(&p).expect("encode");
            let dec = decode_claude_project_dir(&enc).expect("decode");
            assert_eq!(dec, p, "round-trip failed for {}", path_str);
        }
    }

    #[test]
    fn decode_lossy_for_paths_with_dashes_is_documented() {
        // Document the by-design ambiguity: `/tmp/test-ide-tracker` encodes
        // to `-tmp-test-ide-tracker` which decodes back to
        // `/tmp/test/ide/tracker`. The daemon resolves this by then calling
        // `canonicalize()`; if the lossy path doesn't exist on disk we skip
        // the signal. So a real project never warms the wrong root.
        let p = PathBuf::from("/tmp/test-ide-tracker");
        let enc = encode_claude_project_dir(&p).expect("encode");
        let dec = decode_claude_project_dir(&enc).expect("decode");
        assert_eq!(dec, PathBuf::from("/tmp/test/ide/tracker"));
        // The decoded path doesn't exist, so canonicalize() will fail on it
        // in `scan_claude_projects` — the signal is safely dropped.
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert!(decode_claude_project_dir("no-leading-dash").is_none());
        assert!(decode_claude_project_dir("-").is_none());
        assert!(decode_claude_project_dir("").is_none());
    }

    #[test]
    fn parse_session_extracts_cwd_and_dedupes_read_paths_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let f1 = root.join("a.rs");
        let f2 = root.join("b.rs");
        let f3 = root.join("c.rs");
        for f in [&f1, &f2, &f3] {
            std::fs::write(f, "").unwrap();
        }

        let jsonl = root.join("session.jsonl");
        let mut w = std::fs::File::create(&jsonl).unwrap();
        let lines = [
            format!(
                r#"{{"type":"attachment","cwd":"{}","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
                root.display(),
                f1.display()
            ),
            format!(
                r#"{{"type":"attachment","cwd":"{}","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
                root.display(),
                f2.display()
            ),
            // Re-read a.rs (must NOT duplicate)
            format!(
                r#"{{"type":"attachment","cwd":"{}","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
                root.display(),
                f1.display()
            ),
            format!(
                r#"{{"type":"attachment","cwd":"{}","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
                root.display(),
                f3.display()
            ),
            // Non-Read tool — must be skipped (not a Read)
            r#"{"type":"attachment","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#.to_string(),
            // Malformed line — must not crash
            "this is not json".to_string(),
        ];
        for l in lines {
            writeln!(w, "{}", l).unwrap();
        }
        drop(w);

        let parsed = parse_session_jsonl(&jsonl);
        assert_eq!(parsed.cwd.as_deref(), Some(root.to_str().unwrap()));
        let names: Vec<String> = parsed
            .hot_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn parse_session_caps_at_hot_files_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let jsonl = root.join("s.jsonl");
        let mut w = std::fs::File::create(&jsonl).unwrap();
        for i in 0..50 {
            let p = root.join(format!("f{}.rs", i));
            std::fs::write(&p, "").unwrap();
            writeln!(
                w,
                r#"{{"type":"attachment","cwd":"{}","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
                root.display(),
                p.display()
            )
            .unwrap();
        }
        drop(w);

        let parsed = parse_session_jsonl(&jsonl);
        assert!(
            parsed.hot_files.len() <= HOT_FILES_CAP,
            "expected ≤ {} hot files, got {}",
            HOT_FILES_CAP,
            parsed.hot_files.len()
        );
        let last_name = parsed
            .hot_files
            .last()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string());
        assert_eq!(last_name.as_deref(), Some("f49.rs"));
    }

    #[test]
    fn parse_session_keeps_real_cwd_even_when_dir_name_would_be_lossy() {
        // The dir name is `kweli-project` (with a dash). If we relied on
        // decode_claude_project_dir, we'd get `/Users/.../kweli/project`,
        // a phantom path. parse_session_jsonl reads the real cwd field.
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("kweli-project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_root = project_root.canonicalize().unwrap();
        let jsonl = project_root.join("session.jsonl");
        std::fs::write(
            &jsonl,
            format!(
                r#"{{"type":"attachment","cwd":"{}"}}{}"#,
                project_root.display(),
                "\n"
            ),
        )
        .unwrap();

        let parsed = parse_session_jsonl(&jsonl);
        assert_eq!(
            parsed.cwd.as_deref(),
            Some(project_root.to_str().unwrap()),
            "cwd from JSONL is the source of truth, even when dashes ambiguify the dir name",
        );
    }

    #[test]
    fn newest_jsonl_picks_latest_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let p1 = tmp.path().join("old.jsonl");
        let p2 = tmp.path().join("new.jsonl");
        std::fs::write(&p1, "").unwrap();
        std::thread::sleep(Duration::from_millis(15));
        std::fs::write(&p2, "").unwrap();
        let (picked, _) = newest_jsonl(tmp.path()).expect("found");
        assert_eq!(picked.file_name().unwrap(), "new.jsonl");
    }
}
