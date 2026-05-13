//! Multi-tenant search daemon.
//!
//! ## Architecture
//!
//! A single daemon process serves searches for *every* indexed project on the
//! machine. The daemon listens on `~/.cache/ig/daemon.sock` and dispatches
//! incoming queries to a per-root `TenantState` that lives in an LRU cache.
//!
//! Compared to the previous design (one daemon per project), this:
//!   - cuts RAM by ~14× (one process overhead instead of N)
//!   - simplifies install/restart (one systemd unit / launchd plist)
//!   - lets new projects be served the moment they're queried — no preboot
//!
//! ## Wire protocol (one JSON object per line, both directions)
//!
//! Request:
//! ```json
//! { "root": "/abs/path", "pattern": "...", "case_insensitive": false,
//!   "files_only": false, "count_only": false, "context": 0, "type": "rs" }
//! ```
//!
//! Response: the existing `QueryResponse` shape (results / error / candidates).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroUsize;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use lru::LruCache;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use rayon::prelude::*;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::index::ngram::BigramDfTable;
use crate::index::reader::IndexReader;
use crate::index::seal;
use crate::index::writer;
#[cfg(test)]
use crate::query::extract::regex_to_query;
use crate::query::extract::regex_to_query_costed;
use crate::query::plan::NgramQuery;
use crate::search::matcher::{self, SearchConfig};
use crate::util::ig_dir;
use crate::walk::{DEFAULT_EXCLUDES, DEFAULT_MAX_FILE_SIZE};

#[derive(Deserialize)]
struct QueryRequest {
    #[serde(default = "default_op")]
    op: String,
    /// Project root the query applies to (canonical absolute path). The daemon
    /// uses it to locate the right `TenantState` in its LRU cache.
    #[serde(default)]
    root: String,
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default)]
    files_only: bool,
    #[serde(default)]
    count_only: bool,
    #[serde(default = "default_context")]
    context: usize,
    #[serde(rename = "type")]
    file_type: Option<String>,
}

fn default_op() -> String {
    "query".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DaemonResponse {
    #[serde(default)]
    pub results: Option<Vec<DaemonMatch>>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub candidates: usize,
    #[serde(default)]
    pub total_files: usize,
    #[serde(default)]
    pub search_ms: f64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub projects: Option<Vec<ProjectStatus>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonMatch {
    pub file: String,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub count: Option<usize>,
}

fn default_context() -> usize {
    0
}

#[derive(Serialize)]
struct QueryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<MatchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    candidates: usize,
    total_files: usize,
    search_ms: f64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    reloaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    projects: Option<Vec<ProjectStatus>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub root: String,
    pub seconds_since_seen: u64,
    /// True when an agent has called `ig session begin` on this project and
    /// not yet called `ig session end`. While set, the watcher accumulates
    /// dirty paths instead of triggering an overlay rebuild on every batch —
    /// see the `WatchEvent::SessionBegin/End` flow in `watch_worker`.
    #[serde(default)]
    pub session_active: bool,
    /// Number of paths queued during the current session, only meaningful
    /// when `session_active == true`. Reset to 0 on `SessionEnd`.
    #[serde(default)]
    pub session_pending: usize,
    /// What surfaced this project to the daemon's LRU. `None` = an explicit
    /// `ig` invocation (search / warm). `Some("ide-claude")` = proactive
    /// signal from `ide_tracker`. Older daemons omit this field entirely;
    /// `#[serde(default)]` keeps `ig projects list` decode-compatible across
    /// daemon-client version skew.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Size of the most-recent hot-file set for this project (≤
    /// `HOT_FILES_CAP` in `ide_tracker`). `0` when no IDE signal has been
    /// received yet.
    #[serde(default)]
    pub hot_count: usize,
}

#[derive(Serialize)]
struct MatchResult {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
}

// ─── Paths ──────────────────────────────────────────────────────────────────

/// Single global socket. All clients connect here. v1.19.0+ layout: under
/// `cache_root/daemon/`. Migration happens at command entry via
/// `cache::ensure_layout`.
pub fn socket_path() -> PathBuf {
    crate::cache::daemon_dir().join("daemon.sock")
}

fn pid_path() -> PathBuf {
    crate::cache::daemon_dir().join("daemon.pid")
}

fn log_path() -> PathBuf {
    crate::cache::daemon_dir().join("daemon.log")
}

fn lock_path() -> PathBuf {
    crate::cache::daemon_dir().join("daemon.lock")
}

struct DaemonStartLock {
    file: File,
}

impl Drop for DaemonStartLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn acquire_daemon_start_lock() -> Result<Option<DaemonStartLock>> {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create daemon lock dir")?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open daemon lock {}", path.display()))?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(DaemonStartLock { file }));
    }
    let err = std::io::Error::last_os_error();
    let raw = err.raw_os_error();
    if raw == Some(libc::EWOULDBLOCK) || raw == Some(libc::EAGAIN) {
        return Ok(None);
    }
    Err(err).with_context(|| format!("lock daemon {}", path.display()))
}

// ─── Tenant state ───────────────────────────────────────────────────────────

struct ReaderView {
    reader: IndexReader,
    df_table: Option<BigramDfTable>,
    /// Last-observed seal. `None` covers two cases that share the same
    /// semantics: legacy index without a seal, or the daemon has never seen
    /// one. Comparing the full `Seal` (generation + finalized_at_nanos)
    /// rather than just the generation defends against the rare case where
    /// `.ig/` is wiped and rebuilt — the new seal may restart at gen 1 but
    /// `finalized_at_nanos` is monotonic, so the daemon still notices.
    cached_seal: Option<seal::Seal>,
}

/// Per-project state. One per opened project root, kept in an LRU under
/// `GlobalState::tenants`.
struct TenantState {
    reader_view: RwLock<ReaderView>,
    ig_dir: PathBuf,
    root: PathBuf,
    regex_cache: Mutex<LruCache<(String, bool), Arc<regex::bytes::Regex>>>,
    query_cache: Mutex<LruCache<(String, bool), NgramQuery>>,
}

impl TenantState {
    fn open(root: &Path) -> Result<Self> {
        let root = root.to_path_buf();
        let ig = ig_dir(&root);
        let reader = IndexReader::open(&ig).context("open index")?;
        let df_table = if reader.metadata.built_with_idf {
            BigramDfTable::load(&ig)
        } else {
            None
        };
        let cached_seal = seal::read_seal(&ig);
        let cap = NonZeroUsize::new(128).unwrap();
        Ok(Self {
            reader_view: RwLock::new(ReaderView {
                reader,
                df_table,
                cached_seal,
            }),
            ig_dir: ig,
            root,
            regex_cache: Mutex::new(LruCache::new(cap)),
            query_cache: Mutex::new(LruCache::new(cap)),
        })
    }

    /// Reload the reader if the on-disk seal has advanced since the last
    /// observation. One read of a 16-byte file per query — authoritative
    /// because `seal::bump_seal` is the writer's final act (artifacts are
    /// guaranteed already published when the seal changes).
    fn reload_if_changed(&self) -> bool {
        let current = seal::read_seal(&self.ig_dir);
        let needs = {
            let rv = self.reader_view.read().unwrap_or_else(|e| e.into_inner());
            current != rv.cached_seal
        };
        if !needs {
            return false;
        }
        match IndexReader::open(&self.ig_dir) {
            Ok(new_reader) => {
                let new_df = if new_reader.metadata.built_with_idf {
                    BigramDfTable::load(&self.ig_dir)
                } else {
                    None
                };
                let new_count = new_reader.metadata.file_count;
                let old_count = {
                    let mut rv = self.reader_view.write().unwrap_or_else(|e| e.into_inner());
                    let old = rv.reader.metadata.file_count;
                    rv.reader = new_reader;
                    rv.df_table = new_df;
                    rv.cached_seal = current;
                    old
                };
                self.regex_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                self.query_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                eprintln!(
                    "[{}] reloaded: {} → {} files",
                    self.root.display(),
                    old_count,
                    new_count
                );
                true
            }
            Err(e) => {
                eprintln!("[{}] reload failed: {}", self.root.display(), e);
                false
            }
        }
    }
}

// ─── Global state (multi-tenant) ────────────────────────────────────────────

const WATCH_DEBOUNCE: Duration = Duration::from_millis(750);
const MEMORY_GOVERNOR_INTERVAL: Duration = Duration::from_secs(5);
const MEMORY_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);

/// Events flowing into `watch_worker`. The OS watcher emits `Paths(...)`;
/// the IPC `session_begin/end` ops emit the session variants on the same
/// channel so ordering with file events is preserved naturally.
///
/// `SessionEnd` optionally carries a sync acknowledgement sender: when
/// present, the worker fires it after `process_dirty` returns (whether the
/// flush succeeded, was a no-op, or was skipped under memory pressure). The
/// blocking IPC path uses this to guarantee `ig hold end` only returns once
/// the index/seal has been updated.
enum WatchEvent {
    Paths(Vec<PathBuf>),
    SessionBegin,
    SessionEnd(Option<mpsc::SyncSender<()>>),
}

/// Outcome of a blocking session-end IPC call.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SessionEndOutcome {
    /// Worker confirmed the flush completed (or was a no-op).
    Flushed,
    /// We were not the final holder; no flush scheduled.
    NotFinal,
    /// Worker did not ack within the timeout.
    Timeout,
}

/// How long `ig hold end` blocks before giving up on the worker ack.
/// Real flushes on 50–200 files take well under a second; 30 s is a generous
/// upper bound that still prevents an indefinite IPC hang on a wedged watcher.
const SESSION_END_FLUSH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryPressure {
    Normal,
    Soft(u64),
    Hard(u64),
    Unknown,
}

#[derive(Clone, Copy, Debug)]
struct MemoryLimits {
    soft_bytes: u64,
    hard_bytes: u64,
    cooldown: Duration,
}

impl MemoryLimits {
    fn from_config() -> Self {
        let soft_bytes = mb_to_bytes(crate::config::daemon_soft_rss_mb());
        let mut hard_bytes = mb_to_bytes(crate::config::daemon_hard_rss_mb());
        if hard_bytes > 0 && soft_bytes > 0 && hard_bytes < soft_bytes {
            hard_bytes = soft_bytes;
        }
        Self {
            soft_bytes,
            hard_bytes,
            cooldown: Duration::from_secs(crate::config::daemon_cooldown_secs()),
        }
    }

    fn pressure(self, rss_bytes: u64) -> MemoryPressure {
        if self.hard_bytes > 0 && rss_bytes >= self.hard_bytes {
            MemoryPressure::Hard(rss_bytes)
        } else if self.soft_bytes > 0 && rss_bytes >= self.soft_bytes {
            MemoryPressure::Soft(rss_bytes)
        } else {
            MemoryPressure::Normal
        }
    }
}

fn mb_to_bytes(mb: usize) -> u64 {
    (mb as u64).saturating_mul(1024 * 1024)
}

struct ActiveProject {
    root: PathBuf,
    last_seen: Arc<Mutex<Instant>>,
    /// Visible to `ProjectStatus` without locking the worker. Toggled by the
    /// IPC handler *and* by the worker (kept in sync for safety).
    session_active: Arc<AtomicBool>,
    /// Number of concurrently open agent sessions for this project. Claude
    /// Code can have several terminals in the same root; one SessionEnd must
    /// not release the watcher lock while another session is still active.
    session_holders: Arc<AtomicUsize>,
    /// Count of paths queued while the session is open. Updated by the worker.
    session_pending: Arc<AtomicUsize>,
    /// Sender side of the worker channel. Cloned for OS-watcher closures and
    /// for the IPC session ops. `Mutex` only because `mpsc::Sender: !Sync`.
    session_tx: Mutex<mpsc::Sender<WatchEvent>>,
    _watcher: Mutex<RecommendedWatcher>,
    /// Optional FSEvents watcher on `.ig/`. Fires `reload_tenant_if_open`
    /// whenever the `seal` file changes — no rebuild — so external `ig
    /// index` runs from another shell are picked up without waiting for the
    /// next pull check at query time. `None` if `.ig/` did not exist when
    /// the project was activated (the pull path still works).
    _ig_watcher: Mutex<Option<RecommendedWatcher>>,
}

/// Joined onto `ProjectStatus` via `list_projects()` so the `active_projects`
/// map stays a pure tenant table. Set by `record_ide_signal`.
#[derive(Clone)]
struct IdeMetadata {
    source: crate::ide_tracker::IdeSource,
    hot_count: usize,
    #[allow(dead_code)]
    last_signal_at: Instant,
}

struct GlobalState {
    tenants: Mutex<LruCache<PathBuf, Arc<TenantState>>>,
    active_projects: Mutex<HashMap<PathBuf, Arc<ActiveProject>>>,
    idle_timeout: Duration,
    active_projects_max: usize,
    memory_limits: MemoryLimits,
    shutdown_requested: AtomicBool,
    last_pressure_log: Mutex<Option<Instant>>,
    /// Per-root metadata pushed by `ide_tracker`. Joined into ProjectStatus
    /// by `list_projects`. Stays in sync with `active_projects` because
    /// `record_ide_signal` warms the project before inserting.
    ide_metadata: Mutex<HashMap<PathBuf, IdeMetadata>>,
    /// Monotonic counter, surfaced by `ig daemon status` for observability.
    ide_signal_count: AtomicU64,
}

impl GlobalState {
    fn new(max_tenants: usize) -> Self {
        let cap = NonZeroUsize::new(max_tenants.max(1)).unwrap();
        Self {
            tenants: Mutex::new(LruCache::new(cap)),
            active_projects: Mutex::new(HashMap::new()),
            idle_timeout: Duration::from_secs(crate::config::daemon_project_idle_secs()),
            active_projects_max: max_tenants.max(1),
            memory_limits: MemoryLimits::from_config(),
            shutdown_requested: AtomicBool::new(false),
            last_pressure_log: Mutex::new(None),
            ide_metadata: Mutex::new(HashMap::new()),
            ide_signal_count: AtomicU64::new(0),
        }
    }

    /// Consume one [`crate::ide_tracker::IdeSignal`] from the tracker thread.
    /// Warms the project (idempotent) and records the IDE metadata so
    /// `list_projects()` can surface `source`/`hot_count`.
    ///
    /// Called from the dedicated consumer thread; never holds locks across
    /// `warm_project` to avoid stalling other RPCs.
    fn record_ide_signal(self: &Arc<Self>, sig: crate::ide_tracker::IdeSignal) {
        let root = sig.root.clone();
        let source_str = sig.source.as_str();
        let hot = sig.hot_files.len();
        if let Err(e) = self.warm_project(&root) {
            // Memory governor or bad path — log once and move on. The next
            // poll will retry naturally.
            eprintln!(
                "ide-tracker: warm {} failed (source={}): {}",
                root.display(),
                source_str,
                e
            );
            return;
        }
        let canonical = match root.canonicalize() {
            Ok(c) => c,
            Err(_) => root.clone(),
        };
        self.ide_metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                canonical.clone(),
                IdeMetadata {
                    source: sig.source,
                    hot_count: hot,
                    last_signal_at: Instant::now(),
                },
            );
        self.ide_signal_count.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "ide-tracker: signal recorded for {} (source={}, hot={})",
            canonical.display(),
            source_str,
            hot
        );
    }

    fn memory_pressure(&self) -> MemoryPressure {
        current_rss_bytes()
            .map(|rss| self.memory_limits.pressure(rss))
            .unwrap_or(MemoryPressure::Unknown)
    }

    fn enforce_growth_budget(&self, action: &str) -> Result<()> {
        match self.memory_pressure() {
            MemoryPressure::Normal | MemoryPressure::Unknown => Ok(()),
            MemoryPressure::Soft(rss) => {
                self.reclaim_memory(action, rss);
                match self.memory_pressure() {
                    MemoryPressure::Normal | MemoryPressure::Unknown => Ok(()),
                    MemoryPressure::Soft(rss) => anyhow::bail!(
                        "daemon memory soft limit reached during {} (rss={} MB, soft={} MB); background activation paused",
                        action,
                        bytes_to_mb(rss),
                        bytes_to_mb(self.memory_limits.soft_bytes)
                    ),
                    MemoryPressure::Hard(rss) => {
                        self.request_memory_shutdown(action, rss);
                        anyhow::bail!(
                            "daemon memory hard limit reached during {} (rss={} MB, hard={} MB); daemon is shutting down",
                            action,
                            bytes_to_mb(rss),
                            bytes_to_mb(self.memory_limits.hard_bytes)
                        )
                    }
                }
            }
            MemoryPressure::Hard(rss) => {
                self.request_memory_shutdown(action, rss);
                anyhow::bail!(
                    "daemon memory hard limit reached during {} (rss={} MB, hard={} MB); daemon is shutting down",
                    action,
                    bytes_to_mb(rss),
                    bytes_to_mb(self.memory_limits.hard_bytes)
                )
            }
        }
    }

    fn can_run_background_rebuild(&self, action: &str) -> bool {
        match self.enforce_growth_budget(action) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("memory governor: deferring {}: {}", action, e);
                false
            }
        }
    }

    fn enforce_periodic_memory_budget(&self) {
        match self.memory_pressure() {
            MemoryPressure::Normal | MemoryPressure::Unknown => {}
            MemoryPressure::Soft(rss) => self.reclaim_memory("periodic check", rss),
            MemoryPressure::Hard(rss) => self.request_memory_shutdown("periodic check", rss),
        }
    }

    fn reclaim_memory(&self, action: &str, rss: u64) {
        let mut evicted_tenants = 0usize;
        {
            let mut tenants = self.tenants.lock().unwrap_or_else(|e| e.into_inner());
            while tenants.len() > 1 {
                if tenants.pop_lru().is_some() {
                    evicted_tenants += 1;
                } else {
                    break;
                }
            }
        }

        let mut evicted_projects = 0usize;
        {
            let mut projects = self
                .active_projects
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let target = (self.active_projects_max / 2).max(1);
            while projects.len() > target {
                let Some(oldest) = projects
                    .iter()
                    .filter(|(_, p)| !p.session_active.load(Ordering::SeqCst))
                    .max_by_key(|(_, p)| {
                        p.last_seen
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .elapsed()
                    })
                    .map(|(root, _)| root.clone())
                else {
                    break;
                };
                projects.remove(&oldest);
                evicted_projects += 1;
            }
        }

        let mut should_log = false;
        {
            let mut last = self
                .last_pressure_log
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.is_none_or(|t| t.elapsed() >= MEMORY_GOVERNOR_INTERVAL) {
                *last = Some(Instant::now());
                should_log = true;
            }
        }
        if should_log || evicted_tenants > 0 || evicted_projects > 0 {
            eprintln!(
                "memory governor: soft pressure during {} (rss={} MB, soft={} MB); evicted {} tenant(s), {} active project(s)",
                action,
                bytes_to_mb(rss),
                bytes_to_mb(self.memory_limits.soft_bytes),
                evicted_tenants,
                evicted_projects
            );
        }
    }

    fn request_memory_shutdown(&self, action: &str, rss: u64) {
        if self.shutdown_requested.swap(true, Ordering::SeqCst) {
            return;
        }
        write_memory_cooldown(rss, self.memory_limits);
        let _ = std::fs::remove_file(socket_path());
        let _ = std::fs::remove_file(pid_path());
        eprintln!(
            "memory governor: hard limit during {} (rss={} MB, hard={} MB); daemon exits, cooldown={}s",
            action,
            bytes_to_mb(rss),
            bytes_to_mb(self.memory_limits.hard_bytes),
            self.memory_limits.cooldown.as_secs()
        );
        std::thread::spawn(|| {
            std::thread::sleep(MEMORY_SHUTDOWN_GRACE);
            std::process::exit(0);
        });
    }

    /// Get-or-open a tenant for the canonical root path.
    fn tenant_for(&self, root: &Path) -> Result<Arc<TenantState>> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        {
            let mut guard = self.tenants.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = guard.get(&canonical) {
                return Ok(Arc::clone(t));
            }
        }
        self.enforce_growth_budget("open tenant")?;
        let tenant = Arc::new(TenantState::open(&canonical)?);
        {
            let mut guard = self.tenants.lock().unwrap_or_else(|e| e.into_inner());
            guard.put(canonical, Arc::clone(&tenant));
        }
        Ok(tenant)
    }

    fn reload_tenant_if_open(&self, root: &Path) {
        let canonical = match root.canonicalize() {
            Ok(p) => p,
            Err(_) => return,
        };
        let tenant = {
            let mut guard = self.tenants.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(&canonical).map(Arc::clone)
        };
        if let Some(tenant) = tenant {
            tenant.reload_if_changed();
        }
    }

    fn warm_project(self: &Arc<Self>, root: &Path) -> Result<ProjectStatus> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        guard_suspicious_root(&canonical)?;

        if let Some(project) = self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&canonical)
            .cloned()
        {
            *project.last_seen.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            self.touch_index(&canonical);
            return Ok(project.status());
        }

        self.enforce_growth_budget("warm project")?;
        self.catch_up_index(&canonical)?;
        let project = Arc::new(ActiveProject::start(&canonical, Arc::clone(self))?);
        let status = project.status();
        let mut projects = self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if projects.len() >= self.active_projects_max
            && let Some(oldest) = projects
                .iter()
                .max_by_key(|(_, p)| {
                    p.last_seen
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .elapsed()
                })
                .map(|(root, _)| root.clone())
        {
            projects.remove(&oldest);
        }
        projects.insert(canonical, project);
        Ok(status)
    }

    fn catch_up_index(&self, root: &Path) -> Result<()> {
        writer::build_index(root, true, DEFAULT_MAX_FILE_SIZE)?;
        Ok(())
    }

    fn touch_index(&self, root: &Path) {
        crate::cache::touch(&ig_dir(root));
    }

    fn list_projects(&self) -> Vec<ProjectStatus> {
        let ide_meta = self
            .ide_metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut projects: Vec<_> = self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(root, p)| {
                let mut st = p.status();
                if let Some(md) = ide_meta.get(root) {
                    st.source = Some(md.source.as_str().to_string());
                    st.hot_count = md.hot_count;
                }
                st
            })
            .collect();
        projects.sort_by(|a, b| a.root.cmp(&b.root));
        projects
    }

    /// Toggle session mode for `root`. Auto-warms the project if it was not
    /// active, so callers don't need to `ig warm` first. Returns the resulting
    /// `ProjectStatus`.
    fn session_signal(self: &Arc<Self>, root: &Path, begin: bool) -> Result<ProjectStatus> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        guard_suspicious_root(&canonical)?;
        // Ensure the project is warmed (so a worker thread exists to receive
        // the session event). `warm_project` is idempotent for already-active
        // projects and refreshes `last_seen`.
        self.warm_project(&canonical)?;
        let project = self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&canonical)
            .cloned()
            .context("project not active after warm")?;
        project.signal_session(begin);
        Ok(project.status())
    }

    /// Blocking variant of `session_signal(.., false)`: returns once the
    /// worker has flushed the buffered paths (or until `timeout`). Used by
    /// the IPC `session_end` op to make `ig hold end` a real barrier.
    fn session_signal_end_blocking(
        self: &Arc<Self>,
        root: &Path,
        timeout: Duration,
    ) -> Result<(ProjectStatus, SessionEndOutcome)> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        guard_suspicious_root(&canonical)?;
        self.warm_project(&canonical)?;
        let project = self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&canonical)
            .cloned()
            .context("project not active after warm")?;
        let (_active, outcome) = project.signal_session_end_blocking(timeout);
        Ok((project.status(), outcome))
    }

    fn project_status(&self, root: &Path) -> Result<Option<ProjectStatus>> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        Ok(self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&canonical)
            .map(|p| p.status()))
    }

    fn forget_project(&self, root: &Path) -> Result<bool> {
        let canonical = root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", root.display()))?;
        Ok(self
            .active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&canonical)
            .is_some())
    }

    fn prune_idle(&self) {
        let timeout = self.idle_timeout;
        self.active_projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, project| {
                project
                    .last_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed()
                    <= timeout
            });
    }
}

impl ActiveProject {
    fn start(root: &Path, state: Arc<GlobalState>) -> Result<Self> {
        let root = root.to_path_buf();
        let last_seen = Arc::new(Mutex::new(Instant::now()));
        let (tx, rx) = mpsc::channel::<WatchEvent>();
        let session_active = Arc::new(AtomicBool::new(false));
        let session_holders = Arc::new(AtomicUsize::new(0));
        let session_pending = Arc::new(AtomicUsize::new(0));
        // Pre-filter watcher events with the same .ignore / .gitignore /
        // DEFAULT_EXCLUDES rules the indexer uses. Without this, a recursive
        // watcher on a monorepo floods the worker with events from
        // `node_modules/`, `.next/`, `target/`, `var/cache/`, etc. — which the
        // indexer drops at write time, but only after blowing past the
        // OVERLAY_THRESHOLD and triggering full rebuilds in a loop.
        let watcher_ignore = Arc::new(build_watcher_ignore(&root));
        let watched_root = root.clone();
        let watched_ignore = Arc::clone(&watcher_ignore);
        let watcher_tx = tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let paths: Vec<PathBuf> = event
                        .paths
                        .into_iter()
                        .filter(|p| !is_path_ignored(&watched_root, &watched_ignore, p))
                        .collect();
                    if !paths.is_empty() {
                        let _ = watcher_tx.send(WatchEvent::Paths(paths));
                    }
                }
            })
            .context("create file watcher")?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", root.display()))?;

        // Push channel for external rebuilds: a separate watcher on `.ig/`
        // observes the seal file and triggers a tenant reload whenever an
        // out-of-band `ig index` (or another writer) bumps it. Failures are
        // swallowed — the per-query pull check still catches everything.
        let ig = ig_dir(&root);
        let ig_watcher = build_ig_seal_watcher(&root, &ig, Arc::clone(&state));

        let worker_root = root.clone();
        let worker_session = Arc::clone(&session_active);
        let worker_pending = Arc::clone(&session_pending);
        std::thread::spawn(move || {
            watch_worker(worker_root, rx, state, worker_session, worker_pending)
        });

        Ok(Self {
            root,
            last_seen,
            session_active,
            session_holders,
            session_pending,
            session_tx: Mutex::new(tx),
            _watcher: Mutex::new(watcher),
            _ig_watcher: Mutex::new(ig_watcher),
        })
    }

    /// Send a session control event onto the worker channel. Returns the new
    /// active state. Multiple concurrent sessions are reference-counted: only
    /// the first `begin` suspends rebuilds, and only the final `end` flushes.
    ///
    /// Non-blocking: the worker thread handles the flush asynchronously. For
    /// `ig hold end` semantics where the caller must observe the seal bump
    /// before returning, use [`signal_session_end_blocking`] instead.
    fn signal_session(&self, begin: bool) -> bool {
        self.signal_session_inner(begin, None)
    }

    /// Like [`signal_session`] but for the end path: blocks until the worker
    /// acknowledges that the flush completed (or until `timeout` elapses).
    /// Returns the outcome so the IPC layer can surface it to the client.
    fn signal_session_end_blocking(&self, timeout: Duration) -> (bool, SessionEndOutcome) {
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        // The ack is only consumed by the worker when we actually sent a
        // `SessionEnd(Some(tx))` — i.e. when this caller was the final holder.
        let was_final = self.signal_session_inner(false, Some(tx));
        let active = self.session_active.load(Ordering::SeqCst);
        if !was_final {
            return (active, SessionEndOutcome::NotFinal);
        }
        match rx.recv_timeout(timeout) {
            Ok(()) => (active, SessionEndOutcome::Flushed),
            Err(_) => (active, SessionEndOutcome::Timeout),
        }
    }

    /// Returns `true` iff this call was the final holder release (i.e. the
    /// caller can/should expect a flush ack on the provided channel).
    fn signal_session_inner(&self, begin: bool, ack: Option<mpsc::SyncSender<()>>) -> bool {
        if begin {
            let prev = self.session_holders.fetch_add(1, Ordering::SeqCst);
            self.session_active.store(true, Ordering::SeqCst);
            if prev == 0 {
                let _ = self
                    .session_tx
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(WatchEvent::SessionBegin);
            }
            return false;
        }

        let prev = loop {
            let current = self.session_holders.load(Ordering::SeqCst);
            if current == 0 {
                self.session_active.store(false, Ordering::SeqCst);
                return false;
            }
            if self
                .session_holders
                .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break current;
            }
        };

        if prev == 1 {
            // Pre-flip the flag so concurrent `status()` calls see the intent
            // before the worker has consumed the flush event.
            self.session_active.store(false, Ordering::SeqCst);
            let _ = self
                .session_tx
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(WatchEvent::SessionEnd(ack));
            true
        } else {
            false
        }
    }

    fn status(&self) -> ProjectStatus {
        ProjectStatus {
            root: self.root.to_string_lossy().to_string(),
            seconds_since_seen: self
                .last_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed()
                .as_secs(),
            session_active: self.session_active.load(Ordering::SeqCst),
            session_pending: self.session_pending.load(Ordering::SeqCst),
            // Source/hot_count are joined in by GlobalState::list_projects so
            // the IdeMetadata map stays the single source of truth.
            source: None,
            hot_count: 0,
        }
    }
}

/// Maximum number of (path → content-hash) entries kept per project worker.
/// Bounds memory under pathological churn (e.g. monorepos with thousands of
/// hot files); LRU evicts the least recently touched paths.
const WATCH_HASH_CACHE_CAP: usize = 4096;

/// Skip content hashing for files larger than this (5 MB). For big files we
/// always propagate change events — the cost of reading them just to detect a
/// no-op touch outweighs the savings. They're rarely the source of churn.
const WATCH_HASH_MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

fn watch_worker(
    root: PathBuf,
    rx: mpsc::Receiver<WatchEvent>,
    state: Arc<GlobalState>,
    session_active: Arc<AtomicBool>,
    session_pending: Arc<AtomicUsize>,
) {
    // Files queued during a `SessionBegin..SessionEnd` window. They are NOT
    // turned into rebuilds while the session is open — the whole point of
    // sessions is to suppress mid-burst rebuilds. Flushed on SessionEnd.
    let mut session_buffer: Vec<PathBuf> = Vec::new();
    let mut session_open = false;
    let mut hash_cache: LruCache<PathBuf, u64> =
        LruCache::new(NonZeroUsize::new(WATCH_HASH_CACHE_CAP).expect("non-zero cap"));
    while let Ok(ev) = rx.recv() {
        match ev {
            WatchEvent::SessionBegin => {
                session_open = true;
                session_active.store(true, Ordering::SeqCst);
                eprintln!("[{}] session begin — rebuilds suspended", root.display());
                continue;
            }
            WatchEvent::SessionEnd(ack) => {
                session_open = false;
                session_active.store(false, Ordering::SeqCst);
                let count = session_buffer.len();
                eprintln!(
                    "[{}] session end — flushing {} pending paths",
                    root.display(),
                    count
                );
                let mut drained = std::mem::take(&mut session_buffer);
                session_pending.store(0, Ordering::SeqCst);
                if !process_dirty(&root, &mut drained, &mut hash_cache, &state) {
                    session_buffer.extend(drained);
                    session_pending.store(session_buffer.len(), Ordering::SeqCst);
                }
                // Wake up the blocking IPC caller (`ig hold end`) AFTER the
                // flush has completed (or been declined under memory pressure).
                // We send unconditionally: the caller's contract is "I waited
                // for the daemon to finish what it was going to do", not "the
                // index is guaranteed up-to-date". Memory-pressure declines are
                // visible in the daemon log.
                if let Some(tx) = ack {
                    let _ = tx.send(());
                }
                continue;
            }
            WatchEvent::Paths(paths) => {
                let mut dirty: Vec<PathBuf> = paths;
                // Drain the debounce window. Session events stop the drain so
                // they are processed promptly.
                let mut session_end_during_drain = false;
                let mut pending_ack: Option<mpsc::SyncSender<()>> = None;
                while let Ok(ev2) = rx.recv_timeout(WATCH_DEBOUNCE) {
                    match ev2 {
                        WatchEvent::Paths(p) => dirty.extend(p),
                        WatchEvent::SessionBegin => {
                            session_open = true;
                            session_active.store(true, Ordering::SeqCst);
                            eprintln!(
                                "[{}] session begin (mid-batch) — rebuilds suspended",
                                root.display()
                            );
                        }
                        WatchEvent::SessionEnd(ack) => {
                            session_open = false;
                            session_active.store(false, Ordering::SeqCst);
                            session_end_during_drain = true;
                            pending_ack = ack;
                            // Fold any buffered session paths into this batch
                            // so they are flushed together.
                            dirty.append(&mut session_buffer);
                            session_pending.store(0, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                if (session_open || session_active.load(Ordering::SeqCst))
                    && !session_end_during_drain
                {
                    session_buffer.extend(dirty);
                    session_pending.store(session_buffer.len(), Ordering::SeqCst);
                    continue;
                }
                if !session_buffer.is_empty() {
                    dirty.append(&mut session_buffer);
                    session_pending.store(0, Ordering::SeqCst);
                }
                if !process_dirty(&root, &mut dirty, &mut hash_cache, &state) {
                    session_buffer.extend(dirty);
                    session_pending.store(session_buffer.len(), Ordering::SeqCst);
                }
                // If a `SessionEnd` was folded into this drain, ack the
                // blocking IPC caller now that the flush is done.
                if let Some(tx) = pending_ack {
                    let _ = tx.send(());
                }
            }
        }
    }
}

/// Sort/dedupe/hash-filter a dirty batch and run `update_index_for_paths`.
/// Factored out so both the normal and the session-flush paths share logic.
fn process_dirty(
    root: &Path,
    dirty: &mut Vec<PathBuf>,
    hash_cache: &mut LruCache<PathBuf, u64>,
    state: &Arc<GlobalState>,
) -> bool {
    dirty.sort();
    dirty.dedup();
    // Drop paths whose content has not actually changed. Many IDEs and
    // dev-servers `touch` files (mtime bump, identical bytes), which used
    // to re-trigger a full overlay rebuild every time and could pin the
    // daemon at hundreds of % CPU / tens of GB RSS for hours. See the
    // v1.19.4 hotfix and the report in docs/incidents/.
    dirty.retain(|p| should_propagate_change(p, hash_cache));
    if dirty.is_empty() {
        return true;
    }
    if !state.can_run_background_rebuild("watcher rebuild") {
        return false;
    }
    if let Err(e) = writer::update_index_for_paths(root, true, DEFAULT_MAX_FILE_SIZE, dirty) {
        eprintln!("[{}] watcher update failed: {}", root.display(), e);
    } else {
        state.reload_tenant_if_open(root);
    }
    dirty.clear();
    true
}

/// Decide whether a watcher event for `path` reflects a real content change.
///
/// Returns `true` (propagate) when:
///   * the path is gone / unreadable (let the writer tombstone it);
///   * the file is larger than `WATCH_HASH_MAX_FILE_SIZE` (cheap fall-through);
///   * we have never seen this path, or its hash differs from the last seen.
///
/// Returns `false` only when the file exists, is small enough to hash, and
/// produces the same hash as the last known one — i.e. a no-op touch.
fn should_propagate_change(path: &Path, cache: &mut LruCache<PathBuf, u64>) -> bool {
    use std::hash::Hasher;

    let Ok(meta) = std::fs::metadata(path) else {
        cache.pop(path);
        return true;
    };
    if !meta.is_file() {
        return false;
    }
    if meta.len() > WATCH_HASH_MAX_FILE_SIZE {
        return true;
    }
    let Ok(bytes) = std::fs::read(path) else {
        cache.pop(path);
        return true;
    };
    let mut hasher = ahash::AHasher::default();
    hasher.write(&bytes);
    let hash = hasher.finish();
    match cache.get(path) {
        Some(prev) if *prev == hash => false,
        _ => {
            cache.put(path.to_path_buf(), hash);
            true
        }
    }
}

fn is_ig_internal_path(root: &Path, path: &Path) -> bool {
    // Canonicalize both sides so macOS `/var` ↔ `/private/var` symlinks don't
    // make `strip_prefix` fail silently and let `.ig/` events leak through.
    // Mirrors the v1.17.1 fix already applied to writer::normalize_changed_path.
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(&root)
        .ok()
        .is_some_and(|rel| rel.components().any(|c| c.as_os_str() == ".ig"))
}

/// Build a gitignore-style matcher used to drop watcher events for paths the
/// indexer would never index (so we don't waste a rebuild round on them).
///
/// Sources, in order:
///   1. `DEFAULT_EXCLUDES` (node_modules/, target/, .next/, vendor/, …)
///   2. `<root>/.ignore` if present
///   3. `<root>/.gitignore` if present
///
/// Built once at project start; not hot-reloaded. If the user edits their
/// `.ignore` they restart the daemon (or simply re-warm the project after a
/// daemon restart for unrelated reasons).
fn build_watcher_ignore(root: &Path) -> Option<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for dir in DEFAULT_EXCLUDES {
        // Anchor as a directory pattern so `target` matches `target/` at any
        // depth but not files literally named "target".
        let _ = builder.add_line(None, &format!("{dir}/"));
    }
    let project_ignore = root.join(".ignore");
    if project_ignore.is_file() {
        let _ = builder.add(&project_ignore);
    }
    let git_ignore = root.join(".gitignore");
    if git_ignore.is_file() {
        let _ = builder.add(&git_ignore);
    }
    builder.build().ok()
}

/// Decide if a watcher event for `path` should be dropped (ignored).
///
/// Returns `true` when:
///   * the path is inside ig's own internal `.ig/` (legacy local index), or
///   * the path or any of its parents matches the watcher ignore matcher.
fn is_path_ignored(
    root: &Path,
    matcher: &Option<ignore::gitignore::Gitignore>,
    path: &Path,
) -> bool {
    if is_ig_internal_path(root, path) {
        return true;
    }
    let Some(matcher) = matcher else {
        return false;
    };
    // `matched_path_or_any_parents` walks up the path so that an event for
    // `node_modules/foo/bar.ts` is filtered via the `node_modules/` rule on
    // the parent directory.
    matcher.matched_path_or_any_parents(path, false).is_ignore()
}

/// Build an FSEvents watcher on `.ig/` that triggers `reload_tenant_if_open`
/// when the `seal` file changes. Returns `None` if `.ig/` does not exist yet
/// or if the watcher cannot be created (e.g., FSEvents init failure on a
/// remote filesystem). The pull-based seal check inside `reload_if_changed`
/// remains authoritative either way.
fn build_ig_seal_watcher(
    root: &Path,
    ig_dir: &Path,
    state: Arc<GlobalState>,
) -> Option<RecommendedWatcher> {
    if !ig_dir.exists() {
        return None;
    }
    let root_owned = root.to_path_buf();
    let mut watcher =
        notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
            let Ok(event) = res else {
                return;
            };
            // The writer renames `seal.tmp` over `seal` as its final act.
            // Match either the temp or the published name — the rename
            // delivers an event for both on most backends.
            let touched_seal = event.paths.iter().any(|p| {
                matches!(
                    p.file_name().and_then(|n| n.to_str()),
                    Some("seal") | Some("seal.tmp")
                )
            });
            if touched_seal {
                state.reload_tenant_if_open(&root_owned);
            }
        })
        .ok()?;
    watcher.watch(ig_dir, RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}

fn guard_suspicious_root(root: &Path) -> Result<()> {
    if std::env::var("IG_ALLOW_HOME_INDEX").as_deref() == Ok("1") {
        return Ok(());
    }
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut suspicious = vec![
        PathBuf::from("/"),
        PathBuf::from("/usr"),
        PathBuf::from("/home"),
        PathBuf::from("/var"),
        PathBuf::from("/tmp"),
        PathBuf::from("/Users"),
    ];
    if let Some(home) = dirs::home_dir() {
        suspicious.push(home);
    }
    for path in suspicious {
        let path = path.canonicalize().unwrap_or(path);
        if canonical == path {
            anyhow::bail!(
                "refusing to warm {} because it is not a project root",
                canonical.display()
            );
        }
    }
    Ok(())
}

// ─── Server entry points ────────────────────────────────────────────────────

/// Start the global daemon in foreground (this thread blocks accepting
/// connections forever). The path argument is ignored — kept for backwards
/// compatibility with `ig daemon foreground <path>` invocations from old
/// launchd plists.
pub fn start_daemon(_legacy_path: &Path) -> Result<()> {
    if let Some(cooldown) = memory_cooldown_remaining() {
        eprintln!(
            "Daemon (global): memory cooldown active for {}s (rss={} MB, hard={} MB)",
            cooldown.remaining_secs,
            bytes_to_mb(cooldown.rss_bytes),
            bytes_to_mb(cooldown.hard_bytes)
        );
        return Ok(());
    }

    // Make sure the v1.19 layout exists before writing any daemon state.
    // ensure_layout is idempotent and a single stat in the hot path.
    let _ = crate::cache::ensure_layout();
    crate::cache::rotate_daemon_log_if_needed();

    let Some(start_lock) = acquire_daemon_start_lock()? else {
        eprintln!("Daemon (global) already running");
        return Ok(());
    };

    if is_daemon_available() {
        eprintln!("Daemon (global) already running");
        return Ok(());
    }

    purge_legacy_per_project_daemons();

    let max_tenants = crate::config::daemon_max_active_projects();

    let state = Arc::new(GlobalState::new(max_tenants));
    let sock = socket_path();
    if let Some(parent) = sock.parent() {
        std::fs::create_dir_all(parent).context("create cache dir")?;
    }
    let _ = std::fs::remove_file(&sock);

    // Record our PID so `ig daemon status` can find us regardless of how we
    // were launched (systemd unit, launchd, manual `daemon foreground`, …).
    let pid_file = pid_path();
    if let Err(e) = std::fs::write(&pid_file, std::process::id().to_string()) {
        eprintln!("warn: write {} failed: {}", pid_file.display(), e);
    }

    ctrlc_cleanup(sock.clone());

    let listener = UnixListener::bind(&sock).with_context(|| format!("bind {}", sock.display()))?;

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0o600 {}", sock.display()))?;

    eprintln!(
        "Daemon (global) listening on {} — max_tenants={}",
        sock.display(),
        max_tenants
    );

    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                state.prune_idle();
            }
        });
    }
    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(MEMORY_GOVERNOR_INTERVAL);
                state.enforce_periodic_memory_budget();
            }
        });
    }

    // ── IDE tracker: proactively warm projects the user is touching with
    //    Claude Code (and later Cursor / VS Code, v2). Off by default if
    //    IG_IDE_TRACKER_ENABLED=0. The consumer thread drains the channel
    //    and calls record_ide_signal, which warms + records IdeMetadata.
    if crate::ide_tracker::tracker_enabled() {
        let interval = crate::ide_tracker::default_poll_interval();
        let rx = crate::ide_tracker::spawn_tracker(interval);
        let state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("ig-ide-consumer".into())
            .spawn(move || {
                while let Ok(sig) = rx.recv() {
                    state.record_ide_signal(sig);
                }
            })
            .expect("spawn ig-ide-consumer thread");
        eprintln!(
            "IDE tracker enabled: polling ~/.claude/projects/ every {}s",
            interval.as_secs().max(1)
        );
    } else {
        eprintln!("IDE tracker disabled via IG_IDE_TRACKER_ENABLED");
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(stream, &state) {
                        eprintln!("client error: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("accept error: {}", e),
        }
    }

    let _ = std::fs::remove_file(&sock);
    drop(start_lock);
    Ok(())
}

fn handle_client(stream: UnixStream, state: &Arc<GlobalState>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut buf_reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let mut line = String::new();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let response = process_request(&line, state);
        let json = serde_json::to_string(&response)?;
        writeln!(writer, "{}", json)?;
        writer.flush()?;
    }
    Ok(())
}

fn err_response(msg: String) -> QueryResponse {
    QueryResponse {
        results: None,
        error: Some(msg),
        candidates: 0,
        total_files: 0,
        search_ms: 0.0,
        reloaded: false,
        status: None,
        root: None,
        projects: None,
    }
}

fn process_request(line: &str, state: &Arc<GlobalState>) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return err_response(format!("invalid request: {}", e)),
    };

    if req.op == "projects_list" {
        return QueryResponse {
            results: None,
            error: None,
            candidates: 0,
            total_files: 0,
            search_ms: 0.0,
            reloaded: false,
            status: Some("ok".to_string()),
            root: None,
            projects: Some(state.list_projects()),
        };
    }

    if req.root.is_empty() {
        return err_response("missing 'root' field".into());
    }
    let root = PathBuf::from(&req.root);

    if req.op == "warm" {
        return match state.warm_project(&root) {
            Ok(status) => QueryResponse {
                results: None,
                error: None,
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
                status: Some("warmed".to_string()),
                root: Some(status.root),
                projects: None,
            },
            Err(e) => err_response(format!("warm project: {}", e)),
        };
    }

    if req.op == "projects_forget" {
        return match state.forget_project(&root) {
            Ok(true) => QueryResponse {
                results: None,
                error: None,
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
                status: Some("forgotten".to_string()),
                root: Some(root.to_string_lossy().to_string()),
                projects: None,
            },
            Ok(false) => QueryResponse {
                results: None,
                error: None,
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
                status: Some("not_found".to_string()),
                root: Some(root.to_string_lossy().to_string()),
                projects: None,
            },
            Err(e) => err_response(format!("forget project: {}", e)),
        };
    }

    if req.op == "session_begin" || req.op == "session_end" {
        let begin = req.op == "session_begin";
        if begin {
            return match state.session_signal(&root, begin) {
                Ok(status) => QueryResponse {
                    results: None,
                    error: None,
                    candidates: 0,
                    total_files: 0,
                    search_ms: 0.0,
                    reloaded: false,
                    status: Some("session_begin".into()),
                    root: Some(status.root.clone()),
                    projects: Some(vec![status]),
                },
                Err(e) => err_response(format!("{}: {}", req.op, e)),
            };
        }
        // session_end: block until the watcher has flushed and bumped the seal
        // (or timeout). This is what makes `ig hold end` a real barrier: hooks
        // that run a search immediately after won't hit a stale index.
        return match state.session_signal_end_blocking(&root, SESSION_END_FLUSH_TIMEOUT) {
            Ok((status, outcome)) => {
                let status_str = match outcome {
                    SessionEndOutcome::Flushed => "session_end",
                    SessionEndOutcome::NotFinal => "session_end_pending",
                    SessionEndOutcome::Timeout => "session_end_timeout",
                };
                QueryResponse {
                    results: None,
                    error: None,
                    candidates: 0,
                    total_files: 0,
                    search_ms: 0.0,
                    reloaded: false,
                    status: Some(status_str.into()),
                    root: Some(status.root.clone()),
                    projects: Some(vec![status]),
                }
            }
            Err(e) => err_response(format!("session_end: {}", e)),
        };
    }

    if req.op == "session_status" {
        return match state.project_status(&root) {
            Ok(Some(status)) => QueryResponse {
                results: None,
                error: None,
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
                status: Some("ok".into()),
                root: Some(status.root.clone()),
                projects: Some(vec![status]),
            },
            Ok(None) => QueryResponse {
                results: None,
                error: None,
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
                status: Some("inactive".into()),
                root: Some(root.to_string_lossy().to_string()),
                projects: None,
            },
            Err(e) => err_response(format!("session_status: {}", e)),
        };
    }

    if req.op != "query" {
        return err_response(format!("unknown op: {}", req.op));
    }
    if req.pattern.is_empty() {
        return err_response("missing 'pattern' field".into());
    }

    let tenant = match state.tenant_for(&root) {
        Ok(t) => t,
        Err(e) => return err_response(format!("open tenant: {}", e)),
    };

    let _ = state.warm_project(&root);
    let reloaded = tenant.reload_if_changed();
    process_query_cached(&req, &tenant, reloaded)
}

fn process_query_cached(req: &QueryRequest, tenant: &TenantState, reloaded: bool) -> QueryResponse {
    let start = Instant::now();
    let rv = tenant.reader_view.read().unwrap_or_else(|e| e.into_inner());
    let total_files = rv.reader.total_file_count() as usize;
    let cache_key = (req.pattern.clone(), req.case_insensitive);

    let query = {
        let cached = tenant
            .query_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
            .cloned();
        match cached {
            Some(q) => q,
            None => {
                match regex_to_query_costed(
                    &req.pattern,
                    req.case_insensitive,
                    rv.df_table.as_ref(),
                    |query| rv.reader.estimate_query_cost(query),
                ) {
                    Ok(q) => {
                        tenant
                            .query_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .put(cache_key.clone(), q.clone());
                        q
                    }
                    Err(e) => {
                        return QueryResponse {
                            results: None,
                            error: Some(format!("invalid regex: {}", e)),
                            candidates: 0,
                            total_files,
                            search_ms: 0.0,
                            reloaded,
                            status: None,
                            root: None,
                            projects: None,
                        };
                    }
                }
            }
        }
    };

    let candidates = rv.reader.resolve(&query);
    let candidate_count = candidates.len();

    let regex = {
        let cached = tenant
            .regex_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
            .map(Arc::clone);
        match cached {
            Some(r) => r,
            None => match RegexBuilder::new(&req.pattern)
                .case_insensitive(req.case_insensitive)
                .unicode(false)
                .build()
            {
                Ok(r) => {
                    let arc = Arc::new(r);
                    tenant
                        .regex_cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .put(cache_key, Arc::clone(&arc));
                    arc
                }
                Err(e) => {
                    return QueryResponse {
                        results: None,
                        error: Some(format!("regex build error: {}", e)),
                        candidates: candidate_count,
                        total_files,
                        search_ms: start.elapsed().as_secs_f64() * 1000.0,
                        reloaded,
                        status: None,
                        root: None,
                        projects: None,
                    };
                }
            },
        }
    };

    let config = SearchConfig {
        before_context: req.context,
        after_context: req.context,
        count_only: req.count_only,
        files_only: req.files_only,
    };

    let candidate_paths: Vec<(u32, String)> = candidates
        .iter()
        .filter_map(|doc_id| {
            let rel_path = rv.reader.file_path(*doc_id).to_string();
            if let Some(ref ft) = req.file_type
                && !crate::search::indexed::matches_type(&rel_path, ft)
            {
                return None;
            }
            Some((*doc_id, rel_path))
        })
        .collect();

    let root = &tenant.root;

    let results: Vec<MatchResult> = candidate_paths
        .par_iter()
        .map_init(
            || (*regex).clone(),
            |local_re, (_doc_id, rel_path)| match matcher::match_file(
                root, rel_path, local_re, &config,
            ) {
                Ok(Some(file_matches)) => {
                    if req.files_only {
                        Some(vec![MatchResult {
                            file: file_matches.path,
                            line: None,
                            text: None,
                            count: None,
                        }])
                    } else if req.count_only {
                        Some(vec![MatchResult {
                            file: file_matches.path,
                            line: None,
                            text: None,
                            count: Some(file_matches.match_count),
                        }])
                    } else {
                        let matches: Vec<MatchResult> = file_matches
                            .matches
                            .iter()
                            .filter(|m| !m.is_context)
                            .map(|m| MatchResult {
                                file: file_matches.path.clone(),
                                line: Some(m.line_number),
                                text: Some(String::from_utf8_lossy(&m.line).to_string()),
                                count: None,
                            })
                            .collect();
                        if matches.is_empty() {
                            None
                        } else {
                            Some(matches)
                        }
                    }
                }
                _ => None,
            },
        )
        .filter_map(|opt| opt)
        .flatten()
        .collect();

    QueryResponse {
        results: Some(results),
        error: None,
        candidates: candidate_count,
        total_files,
        search_ms: start.elapsed().as_secs_f64() * 1000.0,
        reloaded,
        status: None,
        root: None,
        projects: None,
    }
}

// ─── Background launcher ────────────────────────────────────────────────────

/// Backwards-compat shim. Path is ignored; the global daemon serves all roots.
pub fn start_daemon_background(_legacy_path: &Path) -> Result<()> {
    start_daemon_background_inner(false)
}

pub fn start_daemon_background_silent(_legacy_path: &Path) -> Result<()> {
    start_daemon_background_inner(true)
}

fn start_daemon_background_inner(silent: bool) -> Result<()> {
    if is_daemon_alive() {
        if !silent {
            eprintln!("Daemon (global) already running");
        }
        return Ok(());
    }
    if let Some(cooldown) = memory_cooldown_remaining() {
        if !silent {
            eprintln!(
                "Daemon (global) not started: memory cooldown active for {}s (rss={} MB, hard={} MB)",
                cooldown.remaining_secs,
                bytes_to_mb(cooldown.rss_bytes),
                bytes_to_mb(cooldown.hard_bytes)
            );
        }
        return Ok(());
    }

    let exe = std::env::current_exe().context("get current exe")?;
    let log = log_path();
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log_file = std::fs::File::create(&log).context("create daemon.log")?;
    let log_err = log_file.try_clone()?;

    // We pass a dummy path arg for backwards-compatibility with the CLI parser
    // (`ig daemon foreground <path>`), but the path is ignored at runtime.
    let child = std::process::Command::new(&exe)
        .args(["daemon", "foreground", "/"])
        .env("IG_DAEMON_FOREGROUND", "1")
        .stdout(log_file)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()
        .context("spawn daemon process")?;

    if !silent {
        eprintln!(
            "Daemon (global) started — PID {}, log: {}",
            child.id(),
            log.display()
        );
    }
    Ok(())
}

pub fn stop_daemon(_legacy_path: &Path) -> Result<()> {
    let pid_file = pid_path();
    if pid_file.exists()
        && let Ok(pid_str) = std::fs::read_to_string(&pid_file)
        && let Ok(pid) = pid_str.trim().parse::<i32>()
    {
        // Issue #5: never SIGTERM a PID we can't prove belongs to an ig
        // daemon — after a crash + PID reuse this could kill an unrelated
        // process owned by the user. If the pidfile is stale, just clean up
        // and let the next daemon start cleanly.
        if pid_is_ig_daemon(pid) {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            eprintln!("SIGTERM sent to daemon PID {}", pid);
        } else {
            eprintln!(
                "Stale daemon pidfile (PID {} is not an ig daemon), cleaning up",
                pid
            );
        }
        let _ = std::fs::remove_file(&pid_file);
    }
    let _ = std::fs::remove_file(socket_path());
    Ok(())
}

pub fn daemon_status(_legacy_path: &Path) -> Result<()> {
    let sock = socket_path();
    if sock.exists() && is_daemon_alive() {
        let pid = std::fs::read_to_string(pid_path())
            .unwrap_or_default()
            .trim()
            .to_string();
        let rss = pid
            .parse::<i32>()
            .ok()
            .and_then(process_rss_bytes)
            .map(bytes_to_mb);
        eprintln!(
            "Daemon (global): running (PID {}, socket: {})",
            pid,
            sock.display()
        );
        if let Some(rss) = rss {
            eprintln!(
                "Memory: rss={} MB, soft={} MB, hard={} MB",
                rss,
                crate::config::daemon_soft_rss_mb(),
                crate::config::daemon_hard_rss_mb()
            );
        }
    } else {
        eprintln!("Daemon (global): not running");
        if let Some(cooldown) = memory_cooldown_remaining() {
            eprintln!(
                "Memory cooldown: {}s remaining (rss={} MB, hard={} MB)",
                cooldown.remaining_secs,
                bytes_to_mb(cooldown.rss_bytes),
                bytes_to_mb(cooldown.hard_bytes)
            );
        }
    }
    Ok(())
}

fn bytes_to_mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

fn current_rss_bytes() -> Option<u64> {
    process_rss_bytes(std::process::id() as i32)
}

#[cfg(target_os = "macos")]
fn process_rss_bytes(pid: i32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if ret == size {
        let info = unsafe { info.assume_init() };
        Some(info.pti_resident_size)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn process_rss_bytes(pid: i32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    Some(resident_pages.saturating_mul(page_size as u64))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_rss_bytes(_pid: i32) -> Option<u64> {
    None
}

#[derive(Serialize, Deserialize)]
struct MemoryCooldownFile {
    expires_at: u64,
    rss_bytes: u64,
    hard_bytes: u64,
}

struct MemoryCooldownStatus {
    remaining_secs: u64,
    rss_bytes: u64,
    hard_bytes: u64,
}

fn memory_cooldown_path() -> PathBuf {
    crate::cache::daemon_dir().join("memory.cooldown.json")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write_memory_cooldown(rss_bytes: u64, limits: MemoryLimits) {
    let path = memory_cooldown_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = MemoryCooldownFile {
        expires_at: now_unix_secs().saturating_add(limits.cooldown.as_secs()),
        rss_bytes,
        hard_bytes: limits.hard_bytes,
    };
    if let Ok(json) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, json);
    }
}

fn memory_cooldown_remaining() -> Option<MemoryCooldownStatus> {
    let path = memory_cooldown_path();
    let body = std::fs::read_to_string(&path).ok()?;
    let parsed: MemoryCooldownFile = serde_json::from_str(&body).ok()?;
    let now = now_unix_secs();
    if parsed.expires_at <= now {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(MemoryCooldownStatus {
        remaining_secs: parsed.expires_at - now,
        rss_bytes: parsed.rss_bytes,
        hard_bytes: parsed.hard_bytes,
    })
}

/// Verify a PID belongs to an `ig daemon` process before treating it as
/// "the daemon" (issue #5). Bare `kill(pid, 0)` and SIGTERM without this
/// check would, after a crash + OS PID reuse, signal an unrelated process
/// owned by the same user.
///
/// Platform notes:
///   * Linux: `/proc/<pid>/cmdline` is the cheapest source of truth.
///   * macOS / BSD: shell out to `ps -p <pid> -o command=` (no native
///     equivalent of `/proc` for arbitrary processes without entitlements).
///
/// Returns `false` on any error (missing file, parse failure, ps not in
/// PATH) — callers should treat that as "stale pid, clean up".
fn pid_is_ig_daemon(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let bytes = match std::fs::read(format!("/proc/{}/cmdline", pid)) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let s = String::from_utf8_lossy(&bytes);
        let parts: Vec<&str> = s.split('\0').filter(|p| !p.is_empty()).collect();
        let exe = match parts.first() {
            Some(e) => *e,
            None => return false,
        };
        let basename = exe.rsplit('/').next().unwrap_or(exe);
        basename == "ig" && parts.contains(&"daemon")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = match std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return false,
        };
        let s = String::from_utf8_lossy(&output.stdout);
        let line = s.trim();
        if line.is_empty() {
            return false;
        }
        let mut parts = line.split_whitespace();
        let exe = match parts.next() {
            Some(e) => e,
            None => return false,
        };
        let basename = exe.rsplit('/').next().unwrap_or(exe);
        basename == "ig" && parts.any(|p| p == "daemon")
    }
}

fn is_daemon_alive() -> bool {
    if let Ok(pid_str) = std::fs::read_to_string(pid_path())
        && let Ok(pid) = pid_str.trim().parse::<i32>()
    {
        // Both checks needed: kill(pid, 0)==0 says "a process with this pid
        // exists" but doesn't say it's ours — PID reuse after a crash would
        // otherwise let us latch onto a stranger.
        return unsafe { libc::kill(pid, 0) } == 0 && pid_is_ig_daemon(pid);
    }
    false
}

pub fn is_daemon_available() -> bool {
    is_daemon_alive() && socket_path().exists()
}

// ─── Legacy daemon migration ────────────────────────────────────────────────

/// Kill any stray per-project daemon left over from pre-v1.16.0 binaries
/// and remove their `/tmp/ig-*.sock` sockets. Best-effort, silent on failure.
fn purge_legacy_per_project_daemons() {
    // Find legacy sockets.
    let entries = match std::fs::read_dir("/tmp") {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut killed = 0usize;
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("ig-") && name.ends_with(".sock")) {
            continue;
        }
        let path = entry.path();
        // Find the owning process via lsof if available.
        if let Ok(out) = std::process::Command::new("lsof")
            .args(["-t", &path.to_string_lossy()])
            .output()
            && out.status.success()
        {
            for pid_line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Ok(pid) = pid_line.trim().parse::<i32>() {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    killed += 1;
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        removed += 1;
    }
    if removed > 0 {
        eprintln!(
            "Purged {} legacy per-project socket(s), sent SIGTERM to {} process(es).",
            removed, killed
        );
    }

    // Kill stray pre-v1.19 `ig-rust daemon foreground` processes left over
    // from the shim+backend layout. The new layout ships a single `ig`
    // binary, but a legacy `ig-rust` daemon launched before the migration
    // can survive across reinstalls and squat memory / a stale socket.
    purge_legacy_ig_rust_daemons();
}

/// Kill any leftover `ig-rust daemon …` process (legacy shim+backend layout)
/// or `target/release/ig daemon …` test orphans whose socket lives outside
/// the canonical cache dir. Best-effort, silent on failure.
fn purge_legacy_ig_rust_daemons() {
    let canonical_sock = socket_path();
    // `ps -ax -o pid=,command=` is portable across macOS (BSD ps) and Linux
    // (procps). pgrep's `-a/--full-format` flag is Linux-only and silently
    // returns bare PIDs on macOS, which used to defeat the cmdline match.
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let mut killed = 0usize;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim_start();
        let mut parts = line.splitn(2, ' ');
        let Some(pid_str) = parts.next() else {
            continue;
        };
        let Some(cmd) = parts.next().map(str::trim_start) else {
            continue;
        };
        if !cmd.contains("daemon foreground") {
            continue;
        }
        let Ok(pid) = pid_str.trim().parse::<i32>() else {
            continue;
        };
        if pid == std::process::id() as i32 {
            continue;
        }
        // Only kill if it's clearly a legacy ig-rust binary, or an `ig`
        // process whose socket doesn't match the canonical cache path
        // (i.e. a `target/release/ig daemon` test orphan with TMPDIR set).
        let is_legacy_rust = cmd.contains("/ig-rust ") || cmd.ends_with("/ig-rust");
        let is_test_orphan = cmd.contains("target/release/ig") || cmd.contains("target/debug/ig");
        if !(is_legacy_rust || is_test_orphan) {
            continue;
        }
        // Best-effort: don't touch a process that owns the canonical socket
        // — that would be the real daemon under a non-standard invocation.
        if owns_socket(pid, &canonical_sock) {
            continue;
        }
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        killed += 1;
    }
    if killed > 0 {
        eprintln!("Purged {} legacy/orphan ig daemon process(es).", killed);
    }
}

fn owns_socket(pid: i32, sock: &Path) -> bool {
    let Ok(out) = std::process::Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains(&*sock.to_string_lossy())
}

/// Enumerate every running `ig daemon foreground` process, regardless of
/// whether it lives under `~/.cargo/bin`, `~/.local/bin`, `target/{debug,release}`,
/// or a legacy `ig-rust` path. Excludes the current process.
fn find_ig_daemon_processes() -> Vec<i32> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
    else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    let me = std::process::id() as i32;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim_start();
        let mut parts = line.splitn(2, ' ');
        let Some(pid_str) = parts.next() else {
            continue;
        };
        let Some(cmd) = parts.next().map(str::trim_start) else {
            continue;
        };
        if !cmd.contains("daemon foreground") {
            continue;
        }
        let looks_like_ig = cmd.contains("/ig ")
            || cmd.ends_with("/ig")
            || cmd.contains("/ig-rust")
            || cmd.contains("target/release/ig")
            || cmd.contains("target/debug/ig");
        if !looks_like_ig {
            continue;
        }
        if let Ok(pid) = pid_str.trim().parse::<i32>()
            && pid != me
        {
            pids.push(pid);
        }
    }
    pids
}

/// Post-install/update sanity check. Verifies the daemon ended up in a
/// healthy state and cleans up any stray processes left behind. Returns
/// `Ok(())` only when:
///  * exactly one `ig daemon foreground` process is running,
///  * the canonical socket exists and answers a `projects_list` ping,
///  * the pidfile (if present) matches the running daemon.
///
/// Attempts automatic remediation (kill orphans) before giving up.
pub fn verify_daemon_health() -> Result<()> {
    // 1. Wait briefly for the freshly-bootstrapped daemon to come up.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !is_daemon_available() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if !is_daemon_available() {
        anyhow::bail!("daemon socket not available within 3s of install");
    }

    // 2. Functional ping — make sure the daemon actually answers, not just
    //    that the socket file exists.
    let resp = list_projects_daemon().context("daemon ping (projects_list)")?;
    if let Some(err) = resp.error {
        anyhow::bail!("daemon ping returned error: {}", err);
    }

    // 3. Stray-process check + auto-cleanup.
    let canonical_sock = socket_path();
    let pids = find_ig_daemon_processes();
    if pids.len() > 1 {
        let mut killed = 0usize;
        for pid in &pids {
            if !owns_socket(*pid, &canonical_sock) {
                unsafe {
                    libc::kill(*pid, libc::SIGTERM);
                }
                killed += 1;
            }
        }
        if killed > 0 {
            eprintln!(
                "Health check: cleaned up {} stray ig daemon process(es).",
                killed
            );
            std::thread::sleep(Duration::from_millis(300));
        }
        let pids_after = find_ig_daemon_processes();
        if pids_after.len() > 1 {
            anyhow::bail!(
                "{} ig daemon processes remain after auto-cleanup (PIDs: {:?})",
                pids_after.len(),
                pids_after
            );
        }
    } else if pids.is_empty() {
        anyhow::bail!("daemon socket responds but no ig daemon process found");
    }

    // 4. Pidfile coherence (warn-only — pidfile drift isn't fatal).
    if let Ok(content) = std::fs::read_to_string(pid_path())
        && let Ok(file_pid) = content.trim().parse::<i32>()
    {
        let pids_final = find_ig_daemon_processes();
        if !pids_final.contains(&file_pid) {
            eprintln!(
                "warn: pidfile points to PID {} but running daemon is {:?}",
                file_pid, pids_final
            );
        }
    }

    Ok(())
}

/// Cheap pre-check used by `install_launchd` to decide whether a reinstall
/// is actually needed. Skipping the reload keeps macOS from spamming the
/// "Background items added" Notification Center entry on every `ig update`.
#[cfg(target_os = "macos")]
fn launchd_already_healthy(plist_path: &Path, current_exe: &Path) -> bool {
    if !plist_path.exists() {
        return false;
    }
    // Plist must reference the exe we'd otherwise install. Comparing as
    // string is good enough: install_launchd writes the same Display fmt.
    let Ok(body) = std::fs::read_to_string(plist_path) else {
        return false;
    };
    let exe_str = current_exe.display().to_string();
    if !body.contains(&exe_str) {
        return false;
    }
    // launchctl must report the service as loaded.
    let Ok(out) = std::process::Command::new("launchctl")
        .args(["list", "com.ig.daemon.global"])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    // And the daemon process / socket must respond.
    is_daemon_available()
}

// ─── Service install (systemd-user / launchd) ───────────────────────────────

#[cfg(target_os = "macos")]
pub fn install_launchd(_legacy_path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("get current exe")?;
    let label = "com.ig.daemon.global";
    let plist_dir = dirs::home_dir()
        .context("get home dir")?
        .join("Library/LaunchAgents");
    std::fs::create_dir_all(&plist_dir).context("create LaunchAgents dir")?;
    let plist_path = plist_dir.join(format!("{}.plist", label));
    let log = log_path();

    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>daemon</string>
        <string>foreground</string>
        <string>/</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>"#,
        label = label,
        exe = exe.display(),
        log = log.display(),
    );
    // Idempotent fast-path: if the plist already points to this exe AND the
    // service is loaded AND the daemon socket answers, we don't touch
    // anything. This avoids triggering macOS's "Background items added"
    // Notification Center entry on every `ig update`.
    if launchd_already_healthy(&plist_path, &exe) {
        eprintln!("Daemon already installed and healthy — nothing to do.");
        return Ok(());
    }

    std::fs::write(&plist_path, body).context("write plist")?;

    // Idempotent reload using modern launchctl (bootout/bootstrap). The legacy
    // `load`/`unload` verbs are deprecated on macOS Catalina+ and fail with
    // I/O errors when the agent was loaded into a different domain or when
    // launchd has already started the job. Bootstrapping into `gui/<uid>` is
    // the documented replacement and is supported since OS X Yosemite (2014).
    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{}", uid);
    let service_target = format!("{}/{}", domain, label);

    // Tear down any previous instance of the agent and clean stale state so
    // the freshly-bootstrapped process starts from a known-good baseline.
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &service_target])
        .status();
    let _ = stop_daemon(Path::new("/"));
    let daemon_dir = crate::cache::daemon_dir();
    for stale in ["daemon.pid", "daemon.sock", "daemon.lock"] {
        let _ = std::fs::remove_file(daemon_dir.join(stale));
    }

    // launchd needs a brief moment to fully tear down the previous job before
    // accepting a fresh bootstrap on the same label; otherwise it returns
    // "Input/output error" (EIO). Retry with a short backoff.
    let mut last_code = -1;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(300 * attempt as u64));
            // Repeated bootout in case the previous one was racy/silent.
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &service_target])
                .status();
        }
        let status = std::process::Command::new("launchctl")
            .args(["bootstrap", &domain, &plist_path.to_string_lossy()])
            .status()
            .context("launchctl bootstrap")?;
        if status.success() {
            eprintln!("Installed: {}", plist_path.display());
            eprintln!("Daemon will auto-start on login.");
            // Final sanity check — guarantees the user doesn't end up with
            // multiple ig-daemon processes or a non-responsive socket.
            match verify_daemon_health() {
                Ok(()) => eprintln!("Health check: ✓"),
                Err(e) => return Err(e.context("post-install health check")),
            }
            return Ok(());
        }
        last_code = status.code().unwrap_or(-1);
    }
    anyhow::bail!(
        "launchctl bootstrap failed after retries (exit {})",
        last_code
    );
}

#[cfg(target_os = "macos")]
pub fn uninstall_launchd(_legacy_path: &Path) -> Result<()> {
    let label = "com.ig.daemon.global";
    let plist_path = dirs::home_dir()
        .context("get home dir")?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label));
    if plist_path.exists() {
        let uid = unsafe { libc::getuid() };
        let service_target = format!("gui/{}/{}", uid, label);
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &service_target])
            .status();
        std::fs::remove_file(&plist_path).context("remove plist")?;
        eprintln!("Uninstalled: {}", plist_path.display());
    } else {
        eprintln!("No global plist found");
    }
    stop_daemon(Path::new("/"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install_launchd(_legacy_path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("get current exe")?;
    let unit_dir = dirs::config_dir()
        .context("get config dir")?
        .join("systemd/user");
    std::fs::create_dir_all(&unit_dir).context("create systemd-user dir")?;
    let unit_path = unit_dir.join("ig-daemon.service");
    let log = log_path();
    let body = format!(
        "[Unit]\n\
         Description=ig — global trigram search daemon\n\
         After=default.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} daemon foreground /\n\
         Restart=on-failure\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display(),
        log = log.display(),
    );
    std::fs::write(&unit_path, body).context("write systemd unit")?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "ig-daemon.service"])
        .status()
        .context("systemctl enable --now")?;
    if status.success() {
        eprintln!("Installed: {}", unit_path.display());
        eprintln!("Daemon will auto-start on login.");
    } else {
        eprintln!(
            "systemctl enable --now failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }

    // Idempotent reload: restart so a freshly-installed binary replaces any
    // already-running instance launched from a stale exe path.
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "restart", "ig-daemon.service"])
        .status();

    // Final sanity check — same contract as the macOS path.
    verify_daemon_health().context("post-install health check")?;
    eprintln!("Health check: ✓");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall_launchd(_legacy_path: &Path) -> Result<()> {
    let unit_path = dirs::config_dir()
        .context("get config dir")?
        .join("systemd/user/ig-daemon.service");
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "ig-daemon.service"])
        .status();
    if unit_path.exists() {
        std::fs::remove_file(&unit_path).context("remove unit")?;
        eprintln!("Uninstalled: {}", unit_path.display());
    } else {
        eprintln!("No systemd unit found");
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    stop_daemon(Path::new("/"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install_launchd(_legacy_path: &Path) -> Result<()> {
    anyhow::bail!("daemon install is only implemented for macOS and Linux");
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall_launchd(_legacy_path: &Path) -> Result<()> {
    anyhow::bail!("daemon uninstall is only implemented for macOS and Linux");
}

// ─── Client helpers ─────────────────────────────────────────────────────────

pub fn query_daemon(root: &Path, pattern: &str, case_insensitive: bool) -> Result<String> {
    let canonical = root.canonicalize().context("canonicalize root")?;
    let sock = socket_path();
    let stream = UnixStream::connect(&sock)
        .with_context(|| format!("connect to daemon at {}", sock.display()))?;
    let req = serde_json::json!({
        "root": canonical.to_string_lossy(),
        "pattern": pattern,
        "case_insensitive": case_insensitive,
    });
    let mut writer = &stream;
    writeln!(writer, "{}", req)?;
    writer.flush()?;
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(response)
}

fn request_daemon(req: serde_json::Value) -> Result<Option<DaemonResponse>> {
    let sock = socket_path();
    if !sock.exists() {
        return Ok(None);
    }
    let stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(anyhow::Error::new(e).context("connect daemon")),
    };
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let mut writer = &stream;
    writeln!(writer, "{}", req)?;
    writer.flush()?;
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    let parsed: DaemonResponse =
        serde_json::from_str(&response).context("parse daemon response")?;
    Ok(Some(parsed))
}

pub fn warm_daemon(root: &Path) -> Result<DaemonResponse> {
    if !is_daemon_available() {
        let _ = start_daemon_background_silent(root);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !is_daemon_available() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let canonical = root.canonicalize().context("canonicalize root")?;
    let req = serde_json::json!({
        "op": "warm",
        "root": canonical.to_string_lossy(),
    });
    match request_daemon(req.clone())? {
        Some(resp) if response_needs_newer_daemon(&resp) => {
            restart_daemon_for_protocol_upgrade(root)?;
            request_daemon(req)?.context("daemon is not available")
        }
        Some(resp) => Ok(resp),
        None => anyhow::bail!("daemon is not available"),
    }
}

pub fn list_projects_daemon() -> Result<DaemonResponse> {
    let req = serde_json::json!({ "op": "projects_list" });
    match request_daemon(req.clone())? {
        Some(resp) if response_needs_newer_daemon(&resp) => {
            restart_daemon_for_protocol_upgrade(Path::new("/"))?;
            request_daemon(req)?.context("daemon is not available")
        }
        Some(resp) => Ok(resp),
        None => anyhow::bail!("daemon is not available"),
    }
}

pub fn session_signal_daemon(root: &Path, begin: bool) -> Result<DaemonResponse> {
    if !is_daemon_available() {
        let _ = start_daemon_background_silent(root);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !is_daemon_available() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let canonical = root.canonicalize().context("canonicalize root")?;
    let op = if begin {
        "session_begin"
    } else {
        "session_end"
    };
    let req = serde_json::json!({
        "op": op,
        "root": canonical.to_string_lossy(),
    });
    match request_daemon(req.clone())? {
        Some(resp) if response_needs_newer_daemon(&resp) => {
            restart_daemon_for_protocol_upgrade(root)?;
            request_daemon(req)?.context("daemon is not available")
        }
        Some(resp) => Ok(resp),
        None => anyhow::bail!("daemon is not available"),
    }
}

pub fn session_status_daemon(root: &Path) -> Result<DaemonResponse> {
    let canonical = root.canonicalize().context("canonicalize root")?;
    let req = serde_json::json!({
        "op": "session_status",
        "root": canonical.to_string_lossy(),
    });
    match request_daemon(req.clone())? {
        Some(resp) if response_needs_newer_daemon(&resp) => {
            restart_daemon_for_protocol_upgrade(root)?;
            request_daemon(req)?.context("daemon is not available")
        }
        Some(resp) => Ok(resp),
        None => anyhow::bail!("daemon is not available"),
    }
}

pub fn forget_project_daemon(root: &Path) -> Result<DaemonResponse> {
    let canonical = root.canonicalize().context("canonicalize root")?;
    let req = serde_json::json!({
        "op": "projects_forget",
        "root": canonical.to_string_lossy(),
    });
    match request_daemon(req.clone())? {
        Some(resp) if response_needs_newer_daemon(&resp) => {
            restart_daemon_for_protocol_upgrade(root)?;
            request_daemon(req)?.context("daemon is not available")
        }
        Some(resp) => Ok(resp),
        None => anyhow::bail!("daemon is not available"),
    }
}

fn response_needs_newer_daemon(resp: &DaemonResponse) -> bool {
    resp.error.as_deref().is_some_and(|e| {
        e.contains("invalid request")
            || e.contains("missing field `pattern`")
            || e.contains("unknown op")
    })
}

fn restart_daemon_for_protocol_upgrade(root: &Path) -> Result<()> {
    let _ = stop_daemon(root);
    start_daemon_background_silent(root)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !is_daemon_available() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

pub fn try_query_daemon(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    files_only: bool,
    count_only: bool,
    context: usize,
    file_type: Option<&str>,
) -> Result<Option<DaemonResponse>> {
    let canonical = root.canonicalize().context("canonicalize root")?;
    let mut req = serde_json::json!({
        "op": "query",
        "root": canonical.to_string_lossy(),
        "pattern": pattern,
        "case_insensitive": case_insensitive,
        "files_only": files_only,
        "count_only": count_only,
        "context": context,
    });
    if let Some(ft) = file_type {
        req["type"] = serde_json::Value::String(ft.to_string());
    }
    request_daemon(req)
}

// ─── Signal handler ─────────────────────────────────────────────────────────

fn ctrlc_cleanup(sock_path: PathBuf) {
    let pid = pid_path();
    let _ = ctrlc::set_handler(move || {
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_file(&pid);
        std::process::exit(0);
    });
}

// ─── In-process helper for tests ────────────────────────────────────────────

#[cfg(test)]
fn process_query(line: &str, reader: &IndexReader, root: &Path, reloaded: bool) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return err_response(format!("invalid request: {}", e)),
    };
    let start = Instant::now();
    let total_files = reader.total_file_count() as usize;
    let query = match regex_to_query(&req.pattern, req.case_insensitive, None) {
        Ok(q) => q,
        Err(e) => {
            return QueryResponse {
                results: None,
                error: Some(format!("invalid regex: {}", e)),
                candidates: 0,
                total_files,
                search_ms: 0.0,
                reloaded,
                status: None,
                root: None,
                projects: None,
            };
        }
    };
    let candidates = reader.resolve(&query);
    let candidate_count = candidates.len();
    let regex = match RegexBuilder::new(&req.pattern)
        .case_insensitive(req.case_insensitive)
        .unicode(false)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            return QueryResponse {
                results: None,
                error: Some(format!("regex build error: {}", e)),
                candidates: candidate_count,
                total_files,
                search_ms: start.elapsed().as_secs_f64() * 1000.0,
                reloaded,
                status: None,
                root: None,
                projects: None,
            };
        }
    };
    let config = SearchConfig {
        before_context: req.context,
        after_context: req.context,
        count_only: req.count_only,
        files_only: req.files_only,
    };
    let candidate_paths: Vec<(u32, String)> = candidates
        .iter()
        .filter_map(|doc_id| {
            let rel_path = reader.file_path(*doc_id).to_string();
            if let Some(ref ft) = req.file_type
                && !crate::search::indexed::matches_type(&rel_path, ft)
            {
                return None;
            }
            Some((*doc_id, rel_path))
        })
        .collect();
    let results: Vec<MatchResult> = candidate_paths
        .par_iter()
        .filter_map(|(_doc_id, rel_path)| {
            match matcher::match_file(root, rel_path, &regex, &config) {
                Ok(Some(file_matches)) => {
                    if req.files_only {
                        Some(vec![MatchResult {
                            file: file_matches.path,
                            line: None,
                            text: None,
                            count: None,
                        }])
                    } else if req.count_only {
                        Some(vec![MatchResult {
                            file: file_matches.path,
                            line: None,
                            text: None,
                            count: Some(file_matches.match_count),
                        }])
                    } else {
                        let matches: Vec<MatchResult> = file_matches
                            .matches
                            .iter()
                            .filter(|m| !m.is_context)
                            .map(|m| MatchResult {
                                file: file_matches.path.clone(),
                                line: Some(m.line_number),
                                text: Some(String::from_utf8_lossy(&m.line).to_string()),
                                count: None,
                            })
                            .collect();
                        if matches.is_empty() {
                            None
                        } else {
                            Some(matches)
                        }
                    }
                }
                _ => None,
            }
        })
        .flatten()
        .collect();
    QueryResponse {
        results: Some(results),
        error: None,
        candidates: candidate_count,
        total_files,
        search_ms: start.elapsed().as_secs_f64() * 1000.0,
        reloaded,
        status: None,
        root: None,
        projects: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn setup_test_project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Local .ig/ to avoid races with the shared XDG cache during parallel tests.
        fs::create_dir_all(root.join(".ig")).unwrap();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("main.rs"),
            b"fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        fs::write(
            src.join("lib.rs"),
            b"pub fn greet() -> String {\n    \"world\".to_string()\n}\n",
        )
        .unwrap();
        crate::index::writer::build_index(&root, false, 1_048_576).unwrap();
        (dir, root)
    }

    #[test]
    fn test_seal_bumped_by_full_rebuild() {
        let (dir, root) = setup_test_project();
        let ig = ig_dir(&root);
        // setup_test_project ran build_index which now bumps the seal.
        let g1 = seal::current_generation(&ig);
        assert!(g1 >= 1, "build_index must bump seal, got {}", g1);

        // Add a new source file so update_index_for_paths sees actual work.
        // (A no-op `build_index` short-circuits with "Index is up to date"
        // and intentionally does not bump the seal.)
        fs::write(root.join("src/c.rs"), b"pub fn c() {}\n").unwrap();
        crate::index::writer::build_index(&root, false, 1_048_576).unwrap();

        let g2 = seal::current_generation(&ig);
        assert_eq!(g2, g1 + 1, "rebuild with real work must bump seal");
        drop(dir);
    }

    #[test]
    fn test_reload_if_changed_observes_new_generation() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let tenant = TenantState::open(&canonical).unwrap();

        // First call: no change since open, must return false.
        assert!(!tenant.reload_if_changed());

        // Bump the seal as if a writer just published a rebuild. Reload
        // must observe the new generation on the next call.
        let ig = ig_dir(&canonical);
        let prev = seal::current_generation(&ig);
        seal::bump_seal(&ig).unwrap();
        assert_eq!(seal::current_generation(&ig), prev + 1);
        assert!(
            tenant.reload_if_changed(),
            "must reload after seal generation bump"
        );

        // After reload, cached_seal must match the new on-disk seal.
        let cached = tenant.reader_view.read().unwrap().cached_seal;
        assert_eq!(cached.map(|s| s.generation), Some(prev + 1));
        drop(dir);
    }

    #[test]
    fn test_tenant_open_loads_index() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let tenant = TenantState::open(&canonical).unwrap();
        let rv = tenant.reader_view.read().unwrap();
        assert_eq!(rv.reader.metadata.file_count, 2);
        drop(dir);
    }

    #[test]
    fn test_global_state_caches_tenants() {
        let (dir, root) = setup_test_project();
        let state = GlobalState::new(8);
        let t1 = state.tenant_for(&root).unwrap();
        let t2 = state.tenant_for(&root).unwrap();
        // Same tenant returned (LRU hit).
        assert!(Arc::ptr_eq(&t1, &t2));
        drop(dir);
    }

    #[test]
    fn test_tenant_reload_after_rebuild() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let tenant = TenantState::open(&canonical).unwrap();
        let initial = tenant
            .reader_view
            .read()
            .unwrap()
            .reader
            .metadata
            .file_count;

        // Add a file and rebuild.
        fs::write(root.join("src/extra.rs"), b"pub fn x() {}\n").unwrap();
        // Recreate <root>/.ig/ so ig_dir keeps resolving locally.
        let ig = ig_dir(&canonical);
        let _ = fs::remove_dir_all(&ig);
        fs::create_dir_all(&ig).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        crate::index::writer::build_index(&canonical, false, 1_048_576).unwrap();

        assert!(tenant.reload_if_changed(), "should detect rebuild");
        let after = tenant
            .reader_view
            .read()
            .unwrap()
            .reader
            .metadata
            .file_count;
        assert!(after > initial, "{} should be > {}", after, initial);
        drop(dir);
    }

    #[test]
    fn test_process_query_returns_results() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let ig = ig_dir(&canonical);
        let reader = IndexReader::open(&ig).unwrap();
        let req = format!(
            r#"{{"root":"{}","pattern":"hello","case_insensitive":false}}"#,
            canonical.display()
        );
        let resp = process_query(&req, &reader, &canonical, false);
        assert!(resp.error.is_none(), "{:?}", resp.error);
        assert!(
            resp.results
                .unwrap()
                .iter()
                .any(|m| m.text.as_deref().unwrap_or("").contains("hello"))
        );
        drop(dir);
    }

    #[test]
    fn test_process_request_warm_list_and_forget_project() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let state = Arc::new(GlobalState::new(8));

        let warm = format!(r#"{{"op":"warm","root":"{}"}}"#, canonical.display());
        let resp = process_request(&warm, &state);
        assert_eq!(resp.status.as_deref(), Some("warmed"));
        assert!(resp.error.is_none(), "{:?}", resp.error);

        let list = process_request(r#"{"op":"projects_list"}"#, &state);
        let projects = list.projects.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].root, canonical.to_string_lossy());

        let forget = format!(
            r#"{{"op":"projects_forget","root":"{}"}}"#,
            canonical.display()
        );
        let resp = process_request(&forget, &state);
        assert_eq!(resp.status.as_deref(), Some("forgotten"));

        let list = process_request(r#"{"op":"projects_list"}"#, &state);
        assert!(list.projects.unwrap().is_empty());
        drop(dir);
    }

    #[test]
    fn test_session_signal_is_reference_counted() {
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let state = Arc::new(GlobalState::new(8));

        let status = state.session_signal(&canonical, true).unwrap();
        assert!(status.session_active);

        let status = state.session_signal(&canonical, true).unwrap();
        assert!(
            status.session_active,
            "second begin should keep session active"
        );

        let status = state.session_signal(&canonical, false).unwrap();
        assert!(
            status.session_active,
            "first end must not release another active session"
        );

        let status = state.session_signal(&canonical, false).unwrap();
        assert!(
            !status.session_active,
            "final end should release the session lock"
        );

        let status = state.session_signal(&canonical, false).unwrap();
        assert!(
            !status.session_active,
            "extra end should remain safely inactive"
        );

        drop(dir);
    }

    #[test]
    fn test_session_end_blocking_waits_for_flush() {
        // Regression test for issue #1 from the v1.19.7 review: `ig hold end`
        // (= IPC `session_end`) must NOT return before the watcher has
        // drained the queued paths and bumped the seal. Before the fix, the
        // IPC handler sent SessionEnd on the mpsc channel and immediately
        // replied "session_end", letting a subsequent search hit a stale
        // index.
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let state = Arc::new(GlobalState::new(8));

        // Open a session.
        let _ = state.session_signal(&canonical, true).unwrap();

        // Final-holder release with no buffered paths must report Flushed
        // (worker drained an empty buffer and acked).
        let (status, outcome) = state
            .session_signal_end_blocking(&canonical, Duration::from_secs(5))
            .unwrap();
        assert!(
            !status.session_active,
            "session must be inactive after blocking end"
        );
        assert_eq!(
            outcome,
            SessionEndOutcome::Flushed,
            "blocking end with no holders left must observe the worker ack"
        );

        // Calling blocking end again (with holders already at 0) takes the
        // NotFinal path: no ack is expected and the call returns immediately.
        let (_status, outcome) = state
            .session_signal_end_blocking(&canonical, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            outcome,
            SessionEndOutcome::NotFinal,
            "extra end on a closed session must return NotFinal, not Timeout"
        );

        drop(dir);
    }

    #[test]
    fn test_session_end_blocking_returns_notfinal_when_not_last_holder() {
        // Two `session_begin` → two holders. The first `session_end` is NOT
        // the final release; the blocking variant must short-circuit with
        // NotFinal instead of waiting for an ack that will never come.
        let (dir, root) = setup_test_project();
        let canonical = root.canonicalize().unwrap();
        let state = Arc::new(GlobalState::new(8));

        let _ = state.session_signal(&canonical, true).unwrap();
        let _ = state.session_signal(&canonical, true).unwrap();

        let (status, outcome) = state
            .session_signal_end_blocking(&canonical, Duration::from_secs(1))
            .unwrap();
        assert!(
            status.session_active,
            "session must remain active when another holder is still around"
        );
        assert_eq!(outcome, SessionEndOutcome::NotFinal);

        // Now release the last holder — this one waits for the flush ack.
        let (status, outcome) = state
            .session_signal_end_blocking(&canonical, Duration::from_secs(5))
            .unwrap();
        assert!(!status.session_active);
        assert_eq!(outcome, SessionEndOutcome::Flushed);

        drop(dir);
    }
}
