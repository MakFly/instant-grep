//! XDG cache directory for trigram indexes.
//!
//! By default, indexes are stored under `~/.cache/ig/<hash>/` instead of
//! `<root>/.ig/`. This keeps user projects free of clutter and makes garbage
//! collection trivial (`ig gc`).
//!
//! Backwards compatibility: if `<root>/.ig/` already exists, `ig_dir` keeps
//! using it. Set `IG_LOCAL_INDEX=1` to force local-mode for new projects.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Root of the XDG cache for ig.
pub fn cache_root() -> PathBuf {
    if let Ok(v) = std::env::var("IG_CACHE_DIR") {
        return PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(v).join("ig");
    }
    if let Some(d) = dirs::cache_dir() {
        return d.join("ig");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("ig")
}

/// Stable cache key for a project root: 16 hex chars of SHA-256 over the
/// canonical absolute path. Collisions are astronomically unlikely across one
/// user's projects, and a short name keeps the cache dir easy to inspect.
fn root_hash(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let s = canonical.to_string_lossy();
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let bytes = h.finalize();
    hex_encode(&bytes[..8])
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// XDG-cached index dir for a given root.
pub fn cache_index_dir(root: &Path) -> PathBuf {
    cache_root().join(root_hash(root))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CacheMeta {
    pub root_path: String,
    pub created_at: u64,
    pub last_used_at: u64,
    pub ig_version: String,
}

const META_FILE: &str = "cache-meta.json";

fn meta_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(META_FILE)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True when `cache_dir` is an entry inside the XDG cache (not a local `<root>/.ig/`).
fn is_xdg_entry(cache_dir: &Path) -> bool {
    cache_dir.parent().map(|p| p == cache_root()).unwrap_or(false)
}

/// Write the cache meta file. Idempotent: preserves `created_at` if present.
/// No-op for local `<root>/.ig/` indexes — the meta only matters for entries
/// living under the XDG cache (consumed by `ig gc` and `ig migrate`).
pub fn write_meta(cache_dir: &Path, root: &Path) -> Result<()> {
    if !is_xdg_entry(cache_dir) {
        return Ok(());
    }
    fs::create_dir_all(cache_dir).context("creating cache dir")?;
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let now = now_secs();
    let existing = read_meta(cache_dir).ok();
    let created_at = existing.as_ref().map(|m| m.created_at).unwrap_or(now);
    let m = CacheMeta {
        root_path: canonical.to_string_lossy().into_owned(),
        created_at,
        last_used_at: now,
        ig_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let body = serde_json::to_string_pretty(&m)?;
    fs::write(meta_path(cache_dir), body)?;
    Ok(())
}

/// Read the cache meta file (if present and valid).
pub fn read_meta(cache_dir: &Path) -> Result<CacheMeta> {
    let body = fs::read_to_string(meta_path(cache_dir))?;
    Ok(serde_json::from_str(&body)?)
}

/// Touch `last_used_at` without rewriting the rest. Cheap; called from queries.
/// No-op for local `<root>/.ig/` indexes (no meta to track there).
pub fn touch(cache_dir: &Path) {
    if !is_xdg_entry(cache_dir) {
        return;
    }
    if let Ok(mut m) = read_meta(cache_dir) {
        m.last_used_at = now_secs();
        if let Ok(body) = serde_json::to_string_pretty(&m) {
            let _ = fs::write(meta_path(cache_dir), body);
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub dir: PathBuf,
    pub meta: Option<CacheMeta>,
    pub size_bytes: u64,
}

/// Enumerate every cache entry under `cache_root()`.
pub fn list_entries() -> Result<Vec<CacheEntry>> {
    let root = cache_root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let meta = read_meta(&path).ok();
        let size_bytes = dir_size(&path).unwrap_or(0);
        out.push(CacheEntry {
            dir: path,
            meta,
            size_bytes,
        });
    }
    Ok(out)
}

fn dir_size(p: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(p) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(total)
}

#[derive(Debug, Default)]
pub struct GcReport {
    pub orphan_count: usize,
    pub stale_count: usize,
    pub freed_bytes: u64,
}

/// Garbage-collect the XDG cache.
///
/// - Removes entries whose `root_path` no longer exists (orphans).
/// - If `max_age_days` is set, also removes entries unused for that many days.
/// - `dry_run` reports without deleting.
pub fn gc(max_age_days: Option<u64>, dry_run: bool) -> Result<GcReport> {
    let now = now_secs();
    let mut report = GcReport::default();
    for entry in list_entries()? {
        let mut should_delete = false;
        let mut reason = "";
        match &entry.meta {
            None => {
                should_delete = true;
                reason = "no meta";
                report.orphan_count += 1;
            }
            Some(m) => {
                if !Path::new(&m.root_path).exists() {
                    should_delete = true;
                    reason = "orphan";
                    report.orphan_count += 1;
                } else if let Some(days) = max_age_days {
                    let age_secs = now.saturating_sub(m.last_used_at);
                    if age_secs > days * 86_400 {
                        should_delete = true;
                        reason = "stale";
                        report.stale_count += 1;
                    }
                }
            }
        }
        if should_delete {
            report.freed_bytes += entry.size_bytes;
            let display_root = entry
                .meta
                .as_ref()
                .map(|m| m.root_path.clone())
                .unwrap_or_else(|| "?".to_string());
            eprintln!(
                "{} {} ({}) [{}]",
                if dry_run { "would remove:" } else { "remove:" },
                entry.dir.display(),
                display_root,
                reason,
            );
            if !dry_run {
                let _ = fs::remove_dir_all(&entry.dir);
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Default)]
pub struct MigrateReport {
    pub moved: usize,
    pub skipped: usize,
    pub bytes_moved: u64,
}

/// Migrate `<root>/.ig/` to the XDG cache for one project.
/// No-op if the local `.ig/` doesn't exist.
pub fn migrate_root(root: &Path, dry_run: bool) -> Result<MigrateReport> {
    let mut report = MigrateReport::default();
    let local = root.join(".ig");
    if !local.is_dir() {
        return Ok(report);
    }
    let dest = cache_index_dir(root);
    if dest.exists() {
        eprintln!(
            "skip: cache entry already exists for {} ({})",
            root.display(),
            dest.display()
        );
        report.skipped += 1;
        return Ok(report);
    }
    let size = dir_size(&local).unwrap_or(0);
    eprintln!(
        "{} {} -> {}  ({})",
        if dry_run { "would migrate:" } else { "migrate:" },
        local.display(),
        dest.display(),
        crate::util::format_bytes(size),
    );
    if !dry_run {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // Try fast rename first; fall back to copy + remove if cross-device.
        if fs::rename(&local, &dest).is_err() {
            copy_dir_all(&local, &dest)?;
            fs::remove_dir_all(&local)?;
        }
        write_meta(&dest, root)?;
    }
    report.moved += 1;
    report.bytes_moved += size;
    Ok(report)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Serialize tests that mutate `IG_CACHE_DIR` — the env is process-global
    /// and would otherwise race with parallel test runs reading `cache_root()`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cache_root_honors_ig_cache_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        // SAFETY: test-only env mutation; serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        let r = cache_root();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert_eq!(r, tmp.path());
    }

    #[test]
    fn cache_index_dir_is_deterministic_for_same_root() {
        // Lock so a parallel test isn't toggling IG_CACHE_DIR between the
        // two calls — that would shift the cache_root underneath us.
        let _guard = ENV_LOCK.lock().unwrap();
        let proj = tempdir().unwrap();
        let a = cache_index_dir(proj.path());
        let b = cache_index_dir(proj.path());
        assert_eq!(a, b);
    }

    #[test]
    fn meta_round_trip() {
        // Override cache_root to our tempdir so the entry passes is_xdg_entry().
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        let cache = tmp.path().join("entry");
        let proj = tempdir().unwrap();
        write_meta(&cache, proj.path()).unwrap();
        let m = read_meta(&cache).unwrap();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert!(!m.root_path.is_empty());
        assert!(m.created_at > 0);
    }
}
