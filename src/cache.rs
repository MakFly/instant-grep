//! XDG cache directory for trigram indexes.
//!
//! ## Layout (v1.19.0+)
//!
//! ```text
//! ~/.cache/ig/                       (or ~/Library/Caches/ig/ on macOS)
//! ├── daemon/                        ← daemon runtime state
//! │   ├── daemon.sock
//! │   ├── daemon.pid
//! │   └── daemon.log [.1.gz, .2.gz]  ← rotated at 5 MB, 5 generations kept
//! ├── projects/                      ← per-project caches, hash-keyed
//! │   ├── <hash16>/
//! │   │   ├── lexicon.bin / postings.bin / metadata.bin / overlay_*.bin / seal
//! │   │   └── cache-meta.json
//! │   └── ...
//! ├── by-name/                       ← human-friendly symlinks (read-only)
//! │   ├── tilvest-distribution-app-v2 -> ../projects/2e0c08507bb58341
//! │   └── ...
//! ├── tee/                           ← centralized tee output (was .ig/tee/)
//! │   └── <id>/
//! └── manifest.json                  ← global registry: hash → root, name, size, …
//! ```
//!
//! ## Migration from v1.18.0 and earlier
//!
//! Old layout had hash dirs at the root and `daemon.{sock,pid,log}` mixed in.
//! `ensure_layout()` is called at the entry of every command and migrates
//! automatically. Idempotent. Safe under concurrent ig invocations via a
//! create-only `.layout.lock` file.
//!
//! ## Local mode (still supported)
//!
//! Set `IG_LOCAL_INDEX=1` for new projects to force `<root>/.ig/`. Existing
//! `<root>/.ig/` directories are still recognised for backward compat.

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

/// Where daemon runtime files live (sock, pid, log + rotated logs).
pub fn daemon_dir() -> PathBuf {
    cache_root().join("daemon")
}

/// Where per-project caches live, hash-keyed.
pub fn projects_dir() -> PathBuf {
    cache_root().join("projects")
}

/// Where human-friendly symlinks live: `tilvest-app -> ../projects/<hash>`.
pub fn by_name_dir() -> PathBuf {
    cache_root().join("by-name")
}

/// Centralized tee output (was `<root>/.ig/tee/` per-project before v1.19.0).
pub fn tee_dir() -> PathBuf {
    cache_root().join("tee")
}

/// Global registry: hash → root, name, size, last_used. Cheap `cache-ls`.
pub fn manifest_path() -> PathBuf {
    cache_root().join("manifest.json")
}

const LAYOUT_LOCK: &str = ".layout.lock";
const LAYOUT_MARKER: &str = ".layout-v1";
/// How long to wait for a live lock holder before bailing out (issue #9).
/// Was effectively 5 s with a silent steal; we now wait longer but never
/// steal from a live PID.
const LAYOUT_LOCK_LIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const GC_LOCK: &str = ".gc.lock";
const GC_STAMP: &str = ".gc-last-run";

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

/// XDG-cached index dir for a given root. v1.19.0+ layout: under `projects/`.
pub fn cache_index_dir(root: &Path) -> PathBuf {
    projects_dir().join(root_hash(root))
}

/// True when `cache_dir` is a project entry inside the XDG cache (under
/// `projects/`). Used to gate metadata writes — local `<root>/.ig/`
/// indexes don't get a `cache-meta.json`.
fn is_xdg_entry_v19(cache_dir: &Path) -> bool {
    cache_dir
        .parent()
        .map(|p| p == projects_dir())
        .unwrap_or(false)
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

/// True when `cache_dir` is an entry inside the XDG cache (any layout).
/// Recognises both the v1.19.0+ `projects/<hash>` layout and the pre-v1.19.0
/// `<hash>` layout — the migration in `ensure_layout` runs once at boot but
/// stale references during a long-lived process should not silently no-op.
fn is_xdg_entry(cache_dir: &Path) -> bool {
    is_xdg_entry_v19(cache_dir)
        || cache_dir
            .parent()
            .map(|p| p == cache_root())
            .unwrap_or(false)
}

/// Write the cache meta file. Idempotent: preserves `created_at` if present.
/// No-op for local `<root>/.ig/` indexes — the meta only matters for entries
/// living under the XDG cache (consumed by `ig gc` and `ig migrate`).
///
/// Issue #7: after writing the meta we also (best-effort) refresh the
/// `by-name/` symlink for this project and the global `manifest.json`. Before
/// the fix these were only rebuilt during migration / GC, so a freshly
/// indexed project never appeared under `~/.cache/ig/by-name/` until the
/// next `ig gc` or layout migration ran.
pub fn write_meta(cache_dir: &Path, root: &Path) -> Result<()> {
    if !is_xdg_entry(cache_dir) {
        return Ok(());
    }
    fs::create_dir_all(cache_dir).context("creating cache dir")?;
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let now = now_secs();
    let existing = read_meta(cache_dir).ok();
    let created_at = existing.as_ref().map(|m| m.created_at).unwrap_or(now);
    let was_new = existing.is_none();
    let m = CacheMeta {
        root_path: canonical.to_string_lossy().into_owned(),
        created_at,
        last_used_at: now,
        ig_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let body = serde_json::to_string_pretty(&m)?;
    fs::write(meta_path(cache_dir), body)?;
    // Best-effort refresh. The symlink update is incremental (skip if it
    // already points at us), and the manifest rebuild does an atomic
    // tmp+rename so concurrent index builds don't tear it.
    let _ = ensure_by_name_symlink(cache_dir, &canonical);
    // Only rebuild the full manifest when this entry is brand new — for
    // existing projects only the timestamp changed, which `cache-ls` lazily
    // re-reads via the per-project `cache-meta.json` anyway. Avoids O(projects)
    // work on every overlay rebuild.
    if was_new {
        let _ = rebuild_manifest();
    }
    Ok(())
}

/// Idempotent: create (or leave alone) the `by-name/<slug>` → cache_dir
/// symlink for `root`. Used by `write_meta` so fresh projects show up under
/// `~/.cache/ig/by-name/` without waiting for a full `rebuild_symlinks`.
pub fn ensure_by_name_symlink(cache_dir: &Path, root: &Path) -> Result<()> {
    let by_name = by_name_dir();
    fs::create_dir_all(&by_name).ok();
    let hash = match cache_dir.file_name().and_then(|n| n.to_str()) {
        Some(n) if is_short_hash(n) => n.to_string(),
        _ => return Ok(()),
    };
    let basename = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let mut slug = slugify(basename);
    let mut link_path = by_name.join(&slug);
    let target = Path::new("..").join("projects").join(&hash);

    // If a symlink with this slug already points at our hash, nothing to do.
    if let Ok(existing) = fs::read_link(&link_path)
        && existing == target
    {
        return Ok(());
    }
    // Slug collision with a *different* hash → suffix with first 4 chars of
    // hash to disambiguate, matching `rebuild_symlinks`.
    if link_path.symlink_metadata().is_ok() {
        slug.push('-');
        slug.push_str(&hash[..hash.len().min(4)]);
        link_path = by_name.join(&slug);
        if let Ok(existing) = fs::read_link(&link_path)
            && existing == target
        {
            return Ok(());
        }
        let _ = fs::remove_file(&link_path);
    }
    std::os::unix::fs::symlink(&target, &link_path)
        .with_context(|| format!("symlink {} -> {}", link_path.display(), target.display()))?;
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

/// Enumerate every cache entry. Walks `projects/<hash>/` in the v1.19.0+
/// layout. Falls back to `cache_root/<hash>/` for legacy layouts that
/// `ensure_layout` hasn't migrated yet (e.g., on a brand-new install where
/// `ensure_layout` has not been called by the caller).
pub fn list_entries() -> Result<Vec<CacheEntry>> {
    let mut out = Vec::new();

    let projects = projects_dir();
    if projects.exists() {
        for entry in fs::read_dir(&projects)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !is_short_hash(name) {
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
    }

    // Legacy fallback: hash dirs still at the cache root pre-migration.
    let root = cache_root();
    if root.exists() {
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !is_short_hash(name) {
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
    pub size_pruned_count: usize,
    pub freed_bytes: u64,
}

/// Garbage-collect the XDG cache.
///
/// - Removes entries whose `root_path` no longer exists (orphans).
/// - If `max_age_days` is set, also removes entries unused for that many days.
/// - If `max_size_bytes` is set, also removes least-recently-used entries until
///   the remaining cache is below the cap.
/// - `dry_run` reports without deleting.
pub fn gc(
    max_age_days: Option<u64>,
    max_size_bytes: Option<u64>,
    dry_run: bool,
) -> Result<GcReport> {
    gc_impl(max_age_days, max_size_bytes, dry_run, true)
}

fn gc_impl(
    max_age_days: Option<u64>,
    max_size_bytes: Option<u64>,
    dry_run: bool,
    verbose: bool,
) -> Result<GcReport> {
    let now = now_secs();
    let mut report = GcReport::default();
    let entries = list_entries()?;
    let mut deleted = vec![false; entries.len()];
    let mut removals: Vec<(usize, &'static str)> = Vec::new();
    let mut remaining_size = 0u64;

    for (idx, entry) in entries.iter().enumerate() {
        let mut reason: Option<&'static str> = None;
        match &entry.meta {
            None => {
                reason = Some("no meta");
                report.orphan_count += 1;
            }
            Some(m) => {
                if !Path::new(&m.root_path).exists() {
                    reason = Some("orphan");
                    report.orphan_count += 1;
                } else if let Some(days) = max_age_days {
                    let age_secs = now.saturating_sub(m.last_used_at);
                    if age_secs > days * 86_400 {
                        reason = Some("stale");
                        report.stale_count += 1;
                    }
                }
            }
        }

        if let Some(reason) = reason {
            deleted[idx] = true;
            report.freed_bytes += entry.size_bytes;
            removals.push((idx, reason));
        } else {
            remaining_size = remaining_size.saturating_add(entry.size_bytes);
        }
    }

    if let Some(max_size) = max_size_bytes
        && remaining_size > max_size
    {
        let mut kept: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter_map(|(idx, _)| (!deleted[idx]).then_some(idx))
            .collect();
        kept.sort_by_key(|idx| {
            entries[*idx]
                .meta
                .as_ref()
                .map(|m| m.last_used_at)
                .unwrap_or(0)
        });

        for idx in kept {
            if remaining_size <= max_size {
                break;
            }
            deleted[idx] = true;
            remaining_size = remaining_size.saturating_sub(entries[idx].size_bytes);
            report.freed_bytes = report.freed_bytes.saturating_add(entries[idx].size_bytes);
            report.size_pruned_count += 1;
            removals.push((idx, "size-cap"));
        }
    }

    for (idx, reason) in removals {
        let entry = &entries[idx];
        if verbose {
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
        }
        if !dry_run {
            let _ = fs::remove_dir_all(&entry.dir);
        }
    }

    if !dry_run && (report.orphan_count + report.stale_count + report.size_pruned_count) > 0 {
        let _ = rebuild_symlinks();
        let _ = rebuild_manifest();
    }

    Ok(report)
}

struct CacheLock {
    path: PathBuf,
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn try_cache_lock(name: &str) -> Option<CacheLock> {
    let path = cache_root().join(name);
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
    {
        Ok(_) => Some(CacheLock { path }),
        Err(_) => None,
    }
}

/// Opportunistic auto-GC, called on command startup. It is silent, lock-file
/// protected, and rate-limited so normal searches only pay one cheap stat.
pub fn auto_gc() {
    if !crate::config::cache_auto_gc_enabled() {
        return;
    }
    let root = cache_root();
    if fs::create_dir_all(&root).is_err() {
        return;
    }
    let Some(_lock) = try_cache_lock(GC_LOCK) else {
        return;
    };

    let now = now_secs();
    let stamp = root.join(GC_STAMP);
    if let Ok(body) = fs::read_to_string(&stamp)
        && let Ok(last) = body.trim().parse::<u64>()
    {
        let interval = crate::config::cache_auto_gc_interval_secs();
        if now.saturating_sub(last) < interval {
            return;
        }
    }

    let _ = gc_impl(
        crate::config::cache_auto_gc_days(),
        crate::config::cache_auto_gc_max_size_bytes(),
        false,
        false,
    );
    let _ = fs::write(stamp, now.to_string());
}

pub fn parse_size_bytes(value: &str) -> Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("empty size");
    }

    let split_at = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let (digits, suffix) = value.split_at(split_at);
    if digits.is_empty() {
        anyhow::bail!("invalid size: {value}");
    }
    let n: u64 = digits.parse().context("invalid size number")?;
    let suffix = suffix.trim().to_ascii_lowercase();
    let multiplier = match suffix.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        other => anyhow::bail!("unsupported size suffix: {other}"),
    };
    Ok(n.saturating_mul(multiplier))
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
        if dry_run {
            "would migrate:"
        } else {
            "migrate:"
        },
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

// ─── v1.19.0 layout — ensure / migrate ─────────────────────────────────────

/// Idempotent: ensure the v1.19.0 cache layout exists and migrate any
/// pre-v1.19.0 artifacts into it. Safe under concurrent ig invocations.
///
/// Migration steps when triggered:
/// 1. Acquire `cache_root/.layout.lock` (create-only). On contention, peek
///    for the marker and return if migration already done.
/// 2. Move every `cache_root/<hash16>/` into `cache_root/projects/<hash16>/`.
/// 3. Move `daemon.{sock,pid,log}` from root into `daemon/`.
/// 4. Build `by-name/` symlinks from `projects/*/cache-meta.json`.
/// 5. Write `manifest.json`.
/// 6. Drop the marker `cache_root/.layout-v1` so subsequent calls return fast.
/// 7. Release the lock.
pub fn ensure_layout() -> Result<()> {
    let root = cache_root();
    if root.join(LAYOUT_MARKER).exists() {
        return Ok(()); // hot path
    }
    fs::create_dir_all(&root).context("create cache root")?;

    let lock_path = root.join(LAYOUT_LOCK);
    // Issue #9: the previous "wait 5s then steal" logic could rip the lock
    // out from under a legitimate migration that simply took longer on a
    // big cache. We now record the holder's PID inside the lock file and
    // only steal if either (a) the marker file appears (migration finished
    // under us), or (b) the recorded PID is no longer alive. Live holders
    // get up to LAYOUT_LOCK_LIVE_TIMEOUT before we bail with an error.
    let mut acquired = match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(f) => Some(f),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
        Err(e) => return Err(e).context("create layout lock")?,
    };
    if acquired.is_none() {
        let deadline = std::time::Instant::now() + LAYOUT_LOCK_LIVE_TIMEOUT;
        loop {
            if root.join(LAYOUT_MARKER).exists() {
                return Ok(());
            }
            let holder_alive = fs::read_to_string(&lock_path)
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .filter(|pid| *pid > 1)
                .map(|pid| unsafe { libc::kill(pid, 0) } == 0)
                .unwrap_or(false);
            if !holder_alive {
                let _ = fs::remove_file(&lock_path);
                acquired = Some(
                    fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&lock_path)
                        .context("acquire layout lock after dead-holder takeover")?,
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "layout lock {} held by live PID for > {}s; refusing to steal",
                    lock_path.display(),
                    LAYOUT_LOCK_LIVE_TIMEOUT.as_secs()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    // Record our PID so other processes can tell us apart from a dead holder.
    if let Some(mut f) = acquired {
        use std::io::Write;
        let _ = writeln!(f, "{}", std::process::id());
        drop(f);
    }

    let outcome = (|| -> Result<()> {
        // A pre-v1.19 daemon will keep writing to legacy paths during migration
        // (recreating hash dirs we just moved, holding daemon.sock open). Stop
        // it first; the new-binary daemon starts fresh under daemon/.
        kill_legacy_daemon_if_running();

        fs::create_dir_all(daemon_dir()).context("create daemon dir")?;
        fs::create_dir_all(projects_dir()).context("create projects dir")?;
        fs::create_dir_all(by_name_dir()).context("create by-name dir")?;
        fs::create_dir_all(tee_dir()).context("create tee dir")?;

        migrate_legacy_to_v19()?;
        rebuild_symlinks().ok(); // best-effort
        rebuild_manifest().ok(); // best-effort

        // Drop the marker LAST so partially-completed migrations are retried.
        fs::write(root.join(LAYOUT_MARKER), b"v1\n").context("write layout marker")?;
        Ok(())
    })();

    let _ = fs::remove_file(&lock_path);
    outcome
}

/// Move any pre-v1.19 `<hash>/` and `daemon.{sock,pid,log}` from the root
/// into their new homes. Idempotent; tolerates partial states. When a hash
/// dir exists at both the legacy root and `projects/`, the newer one wins
/// (handles the case where a stale v1.18 daemon recreated entries during a
/// previous migration attempt).
fn migrate_legacy_to_v19() -> Result<()> {
    let root = cache_root();
    let projects = projects_dir();
    let daemon = daemon_dir();

    for entry in fs::read_dir(&root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        // Skip the new-layout directories themselves.
        if matches!(name, "daemon" | "projects" | "by-name" | "tee") {
            continue;
        }
        if name.starts_with('.') || name == "manifest.json" {
            continue;
        }

        if path.is_dir() && is_short_hash(name) {
            let dest = projects.join(name);
            if !dest.exists() {
                let _ = fs::rename(&path, &dest);
            } else if newer_than(&path, &dest) {
                // Legacy is newer (a pre-v1.19 daemon kept writing to the old
                // path after a partial migration). Replace stale dest with it.
                let _ = fs::remove_dir_all(&dest);
                let _ = fs::rename(&path, &dest);
            } else {
                // Dest already covers this entry — drop the legacy.
                let _ = fs::remove_dir_all(&path);
            }
            continue;
        }

        if matches!(name, "daemon.sock" | "daemon.pid" | "daemon.log") && path.is_file() {
            let dest = daemon.join(name);
            if !dest.exists() {
                let _ = fs::rename(&path, &dest);
            } else {
                // Daemon files at both locations: drop the legacy. The
                // running new-layout daemon is the source of truth.
                let _ = fs::remove_file(&path);
            }
            continue;
        }
    }
    Ok(())
}

/// SIGTERM any pre-v1.19 daemon whose PID file still lives at the cache
/// root. The new-layout daemon's PID is in `daemon/daemon.pid`; the old one
/// was at `cache_root/daemon.pid`. Best-effort, no error if process is gone.
fn kill_legacy_daemon_if_running() {
    let pid_file = cache_root().join("daemon.pid");
    let pid: Option<i32> = fs::read_to_string(&pid_file)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let Some(pid) = pid else { return };
    if pid <= 1 {
        return;
    }
    // SAFETY: kill(pid, SIGTERM) is safe; the FFI takes raw integers and
    // returns -1/errno on failure (we ignore both).
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    // Brief wait so the daemon releases its file handles before we rename.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = fs::remove_file(&pid_file);
}

fn newer_than(a: &Path, b: &Path) -> bool {
    let ma = fs::metadata(a)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mb = fs::metadata(b)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    ma > mb
}

fn is_short_hash(name: &str) -> bool {
    name.len() == 16 && name.chars().all(|c| c.is_ascii_hexdigit())
}

/// Rebuild `by-name/` symlinks. Idempotent: removes any stale symlinks first
/// (anything in `by-name/` that doesn't resolve to a `projects/*` entry).
pub fn rebuild_symlinks() -> Result<()> {
    let projects = projects_dir();
    let by_name = by_name_dir();
    if !projects.exists() {
        return Ok(());
    }
    fs::create_dir_all(&by_name).ok();

    // Wipe stale symlinks first.
    if let Ok(rd) = fs::read_dir(&by_name) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.symlink_metadata()
                .map(|m| m.is_symlink())
                .unwrap_or(false)
            {
                let _ = fs::remove_file(&p);
            }
        }
    }

    let mut used_slugs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in fs::read_dir(&projects)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let hash = match dir.file_name().and_then(|n| n.to_str()) {
            Some(n) if is_short_hash(n) => n.to_string(),
            _ => continue,
        };
        let meta = match read_meta(&dir) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let basename = Path::new(&meta.root_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();
        let mut slug = slugify(&basename);
        if used_slugs.contains(&slug) {
            // Suffix with first 4 hex chars of hash to disambiguate.
            slug.push('-');
            slug.push_str(&hash[..4]);
        }
        used_slugs.insert(slug.clone());

        let link_path = by_name.join(&slug);
        let target = Path::new("..").join("projects").join(&hash);
        let _ = std::os::unix::fs::symlink(&target, &link_path);
    }
    Ok(())
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ManifestEntry {
    pub hash: String,
    pub root: String,
    pub name: String,
    pub size_bytes: u64,
    pub last_used_at: u64,
    pub ig_version: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Manifest {
    pub version: u32,
    pub ig_version: String,
    pub updated_at: u64,
    pub entries: Vec<ManifestEntry>,
}

/// Rewrite `manifest.json` from the on-disk projects/ tree. Atomic via tmp+rename.
pub fn rebuild_manifest() -> Result<()> {
    let projects = projects_dir();
    let mut entries = Vec::new();
    if projects.exists() {
        for e in fs::read_dir(&projects)? {
            let dir = match e {
                Ok(e) => e.path(),
                Err(_) => continue,
            };
            if !dir.is_dir() {
                continue;
            }
            let hash = match dir.file_name().and_then(|n| n.to_str()) {
                Some(n) if is_short_hash(n) => n.to_string(),
                _ => continue,
            };
            let meta = match read_meta(&dir) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size_bytes = dir_size(&dir).unwrap_or(0);
            let name = Path::new(&meta.root_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();
            entries.push(ManifestEntry {
                hash,
                root: meta.root_path,
                name,
                size_bytes,
                last_used_at: meta.last_used_at,
                ig_version: meta.ig_version,
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.last_used_at));

    let manifest = Manifest {
        version: 1,
        ig_version: env!("CARGO_PKG_VERSION").to_string(),
        updated_at: now_secs(),
        entries,
    };
    let body = serde_json::to_string_pretty(&manifest)?;
    let path = manifest_path();
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).context("write manifest.json.tmp")?;
    fs::rename(&tmp, &path).context("publish manifest.json")?;
    Ok(())
}

/// Read the manifest. Returns `None` if missing or malformed (caller should
/// fall back to a `list_entries()` walk). Public helper for future
/// `ig cache-ls`-style consumers — currently the writer side rebuilds the
/// manifest opportunistically and CLI commands walk `projects/` directly.
#[allow(dead_code)]
pub fn read_manifest() -> Option<Manifest> {
    let body = fs::read_to_string(manifest_path()).ok()?;
    serde_json::from_str(&body).ok()
}

/// Rotate `daemon.log` if it exceeds 5 MB. Keeps last 5 raw rotations.
/// Best-effort: errors are swallowed (logging shouldn't take down the daemon).
pub fn rotate_daemon_log_if_needed() {
    const MAX_BYTES: u64 = 5 * 1024 * 1024;
    const KEEP: usize = 5;

    let log = daemon_dir().join("daemon.log");
    let size = match fs::metadata(&log) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size < MAX_BYTES {
        return;
    }

    // Drop the oldest if we'd otherwise exceed KEEP.
    let oldest = log.with_file_name(format!("daemon.log.{}", KEEP));
    let _ = fs::remove_file(&oldest);

    // Shift older rotations: .{KEEP-1} → .{KEEP}, …, .1 → .2.
    for i in (1..KEEP).rev() {
        let src = log.with_file_name(format!("daemon.log.{}", i));
        let dst = log.with_file_name(format!("daemon.log.{}", i + 1));
        if src.exists() {
            let _ = fs::rename(&src, &dst);
        }
    }

    // Move current log → .1, then truncate the live file.
    let _ = fs::rename(&log, log.with_file_name("daemon.log.1"));
    let _ = fs::write(&log, b"");
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
        // v1.19 layout: meta entries live under projects/<hash>/.
        let cache = projects_dir().join("entry");
        let proj = tempdir().unwrap();
        fs::create_dir_all(&cache).unwrap();
        write_meta(&cache, proj.path()).unwrap();
        let m = read_meta(&cache).unwrap();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert!(!m.root_path.is_empty());
        assert!(m.created_at > 0);
    }

    #[test]
    fn ensure_layout_creates_v19_dirs() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        ensure_layout().unwrap();
        let exists_marker = tmp.path().join(LAYOUT_MARKER).exists();
        let daemon_exists = daemon_dir().exists();
        let projects_exists = projects_dir().exists();
        let by_name_exists = by_name_dir().exists();
        let tee_exists = tee_dir().exists();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert!(exists_marker, "layout marker must be written");
        assert!(daemon_exists);
        assert!(projects_exists);
        assert!(by_name_exists);
        assert!(tee_exists);
    }

    #[test]
    fn ensure_layout_migrates_legacy_hash_dirs() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        // Plant a fake legacy entry: cache_root/<hash16>/ with a marker file.
        let legacy_hash = "deadbeefcafebabe";
        let legacy_dir = tmp.path().join(legacy_hash);
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(legacy_dir.join("metadata.bin"), b"x").unwrap();

        // Plant a legacy daemon.log at the root too.
        fs::write(tmp.path().join("daemon.log"), b"old log").unwrap();

        ensure_layout().unwrap();

        let migrated_dir = projects_dir().join(legacy_hash);
        let migrated_log = daemon_dir().join("daemon.log");
        let still_at_root = legacy_dir.exists();
        let log_at_root = tmp.path().join("daemon.log").exists();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert!(
            migrated_dir.exists(),
            "hash dir must move to projects/<hash>"
        );
        assert!(
            migrated_log.exists(),
            "daemon.log must move to daemon/daemon.log"
        );
        assert!(!still_at_root, "old hash dir must be gone from cache_root");
        assert!(!log_at_root, "old daemon.log must be gone from cache_root");
    }

    #[test]
    fn ensure_layout_is_idempotent() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        ensure_layout().unwrap();
        let first_run = std::fs::metadata(tmp.path().join(LAYOUT_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_layout().unwrap();
        let second_run = std::fs::metadata(tmp.path().join(LAYOUT_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert_eq!(
            first_run, second_run,
            "layout marker must not be rewritten on subsequent calls"
        );
    }

    #[test]
    fn rebuild_symlinks_creates_human_names() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        ensure_layout().unwrap();

        // Plant a fake project entry under projects/.
        let hash = "abcdef0123456789";
        let entry = projects_dir().join(hash);
        fs::create_dir_all(&entry).unwrap();
        let proj = tempdir().unwrap();
        let proj_dir = proj.path().join("my-cool-app");
        fs::create_dir_all(&proj_dir).unwrap();
        write_meta(&entry, &proj_dir).unwrap();

        rebuild_symlinks().unwrap();
        let link = by_name_dir().join("my-cool-app");
        let exists = link.exists();
        let target = link.read_link().ok();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert!(exists, "by-name symlink must exist");
        assert_eq!(target, Some(Path::new("..").join("projects").join(hash)));
    }

    #[test]
    fn rebuild_manifest_writes_entries() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        ensure_layout().unwrap();

        let hash = "0123456789abcdef";
        let entry = projects_dir().join(hash);
        fs::create_dir_all(&entry).unwrap();
        let proj = tempdir().unwrap();
        write_meta(&entry, proj.path()).unwrap();

        rebuild_manifest().unwrap();
        let m = read_manifest().expect("manifest must exist");
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert_eq!(m.version, 1);
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].hash, hash);
    }

    #[test]
    fn parse_size_bytes_accepts_common_suffixes() {
        assert_eq!(parse_size_bytes("42").unwrap(), 42);
        assert_eq!(parse_size_bytes("2KB").unwrap(), 2 * 1024);
        assert_eq!(parse_size_bytes("3mb").unwrap(), 3 * 1024 * 1024);
        assert_eq!(parse_size_bytes("4GiB").unwrap(), 4 * 1024 * 1024 * 1024);
        assert!(parse_size_bytes("12tb").is_err());
    }

    #[test]
    fn gc_prunes_oldest_entries_to_size_cap() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("IG_CACHE_DIR", tmp.path());
        }
        ensure_layout().unwrap();

        let old_root = tempdir().unwrap();
        let new_root = tempdir().unwrap();
        let old_entry = projects_dir().join("1111111111111111");
        let new_entry = projects_dir().join("2222222222222222");
        fs::create_dir_all(&old_entry).unwrap();
        fs::create_dir_all(&new_entry).unwrap();
        fs::write(old_entry.join("postings.bin"), vec![1u8; 1024]).unwrap();
        fs::write(new_entry.join("postings.bin"), vec![2u8; 1024]).unwrap();
        write_meta(&old_entry, old_root.path()).unwrap();
        write_meta(&new_entry, new_root.path()).unwrap();

        let mut old_meta = read_meta(&old_entry).unwrap();
        old_meta.last_used_at = 10;
        fs::write(
            meta_path(&old_entry),
            serde_json::to_string_pretty(&old_meta).unwrap(),
        )
        .unwrap();
        let mut new_meta = read_meta(&new_entry).unwrap();
        new_meta.last_used_at = 20;
        fs::write(
            meta_path(&new_entry),
            serde_json::to_string_pretty(&new_meta).unwrap(),
        )
        .unwrap();

        let entries = list_entries().unwrap();
        let newest_size = entries
            .iter()
            .find(|e| e.dir == new_entry)
            .map(|e| e.size_bytes)
            .unwrap();

        let report = gc(None, Some(newest_size), false).unwrap();
        let old_exists = old_entry.exists();
        let new_exists = new_entry.exists();
        unsafe {
            std::env::remove_var("IG_CACHE_DIR");
        }
        assert_eq!(report.size_pruned_count, 1);
        assert!(!old_exists, "least-recently-used entry should be pruned");
        assert!(new_exists, "newest entry should remain");
    }
}
