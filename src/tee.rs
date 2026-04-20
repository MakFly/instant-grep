//! Tee fallback store — keeps the raw output of failed/truncated commands on
//! disk so an agent can consult the full stack trace without re-running.
//!
//! Files live under `~/.local/share/ig/tee/<timestamp>_<slug>.log` (macOS:
//! `~/Library/Application Support/ig/tee/`). Each file is capped at 1 MiB and
//! the directory is pruned to the 20 most recent entries. Mode is `0o600`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const TEE_MAX_FILES: usize = 20;
const TEE_MAX_BYTES: usize = 1024 * 1024;
const TEE_MIN_BYTES: usize = 500;
const TEE_MIN_RATIO: f64 = 2.0;
const TEE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A single tee entry listed on disk.
pub struct TeeEntry {
    pub id: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(PartialEq, Eq)]
enum TeeMode {
    Always,
    Never,
    Failures,
}

fn tee_mode() -> TeeMode {
    match std::env::var("IG_TEE").ok().as_deref() {
        Some("always") => TeeMode::Always,
        Some("never") | Some("0") => TeeMode::Never,
        _ => TeeMode::Failures,
    }
}

/// Base directory for tee files. Created on first write.
pub fn tee_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from)?;
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/ig/tee")
    } else {
        home.join(".local/share/ig/tee")
    };
    Some(base)
}

/// Decide whether the raw output should be teed to disk.
///
/// Default (`Failures` mode): only when the command failed, its raw output
/// exceeds `TEE_MIN_BYTES` and the filter compressed it by more than `2x`.
pub fn should_save(raw_len: usize, filtered_len: usize, exit_code: i32) -> bool {
    match tee_mode() {
        TeeMode::Never => false,
        TeeMode::Always => raw_len > TEE_MIN_BYTES,
        TeeMode::Failures => {
            if exit_code == 0 || raw_len <= TEE_MIN_BYTES {
                return false;
            }
            let denom = filtered_len.max(1) as f64;
            (raw_len as f64 / denom) > TEE_MIN_RATIO
        }
    }
}

fn slugify(cmd: &str) -> String {
    let mut out = String::with_capacity(40);
    let mut last_dash = false;
    for c in cmd.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("cmd");
    }
    out
}

/// Save raw output (capped at `TEE_MAX_BYTES`) and return the tee id.
/// Returns `None` if the tee directory cannot be written or `HOME` is unset.
pub fn save(raw: &[u8], cmd: &str) -> Option<String> {
    save_in(&tee_dir()?, raw, cmd)
}

/// Test-friendly variant: save into an explicit directory.
pub fn save_in(dir: &Path, raw: &[u8], cmd: &str) -> Option<String> {
    fs::create_dir_all(dir).ok()?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));

    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let id = format!("{}_{}", ts, slugify(cmd));
    let path = dir.join(format!("{}.log", id));

    let capped: &[u8] = if raw.len() > TEE_MAX_BYTES {
        &raw[..TEE_MAX_BYTES]
    } else {
        raw
    };

    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    f.write_all(capped).ok()?;
    f.flush().ok()?;

    rotate(dir);
    Some(id)
}

/// Read the raw content of a tee entry by id (without the `.log` suffix).
pub fn read(id: &str) -> Option<Vec<u8>> {
    read_in(&tee_dir()?, id)
}

/// Test-friendly variant: read from an explicit directory.
pub fn read_in(dir: &Path, id: &str) -> Option<Vec<u8>> {
    fs::read(dir.join(format!("{}.log", id))).ok()
}

/// List all tee entries, newest first.
pub fn list() -> Vec<TeeEntry> {
    match tee_dir() {
        Some(d) => list_in(&d),
        None => vec![],
    }
}

/// Test-friendly variant: list entries of an explicit directory.
pub fn list_in(dir: &Path) -> Vec<TeeEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut out: Vec<TeeEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let file_name = path.file_name()?.to_string_lossy().into_owned();
            let id = file_name.strip_suffix(".log")?.to_string();
            let meta = e.metadata().ok()?;
            Some(TeeEntry {
                id,
                path,
                bytes: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.modified));
    out
}

/// Delete every tee entry. Returns the number of files removed.
pub fn clear() -> usize {
    let mut removed = 0usize;
    for entry in list() {
        if fs::remove_file(&entry.path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Prune the directory: keep the `TEE_MAX_FILES` most recent entries and
/// remove anything older than `TEE_TTL`.
fn rotate(dir: &Path) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut entries: Vec<(PathBuf, SystemTime)> = read_dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("log") {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();
    entries.sort_by_key(|b| std::cmp::Reverse(b.1));

    for (idx, (path, modified)) in entries.iter().enumerate() {
        let too_old = now
            .duration_since(*modified)
            .map(|age| age > TEE_TTL)
            .unwrap_or(false);
        if idx >= TEE_MAX_FILES || too_old {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize the subset of tests that mutate the process-wide `IG_TEE`
    /// environment variable. File-system tests use their own tempdir so they
    /// are already isolated.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn slugify_is_safe() {
        assert_eq!(slugify("cargo test --release"), "cargo-test-release");
        assert_eq!(slugify("   !! ??  "), "cmd");
        assert_eq!(slugify("git diff HEAD~1"), "git-diff-head-1");
    }

    #[test]
    fn should_save_respects_mode_failures() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("IG_TEE") };
        assert!(!should_save(10_000, 1_000, 0));
        assert!(!should_save(100, 50, 1));
        assert!(!should_save(1500, 1000, 1));
        assert!(should_save(4000, 1000, 1));
    }

    #[test]
    fn should_save_always_and_never() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("IG_TEE", "always") };
        assert!(should_save(1000, 900, 0));
        unsafe { std::env::set_var("IG_TEE", "never") };
        assert!(!should_save(100_000, 10, 1));
        unsafe { std::env::remove_var("IG_TEE") };
    }

    #[test]
    fn save_then_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = b"hello world, this is a long trace\n".repeat(50);
        let id = save_in(tmp.path(), &raw, "cargo test").expect("save");
        let back = read_in(tmp.path(), &id).expect("read");
        assert_eq!(back, raw);
    }

    #[test]
    fn rotation_keeps_20_most_recent() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..25 {
            save_in(tmp.path(), b"x".repeat(1000).as_slice(), &format!("cmd-{}", i)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let entries = list_in(tmp.path());
        assert!(
            entries.len() <= TEE_MAX_FILES,
            "expected <= {} entries, got {}",
            TEE_MAX_FILES,
            entries.len()
        );
    }

    #[test]
    fn saved_file_has_0o600_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let id = save_in(tmp.path(), b"secret output", "cargo build").unwrap();
        let path = tmp.path().join(format!("{}.log", id));
        let meta = fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn truncation_at_max_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let huge = vec![b'A'; TEE_MAX_BYTES + 100];
        let id = save_in(tmp.path(), &huge, "huge").unwrap();
        let back = read_in(tmp.path(), &id).unwrap();
        assert_eq!(back.len(), TEE_MAX_BYTES);
    }
}
