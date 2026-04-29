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

use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroUsize;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use lru::LruCache;
use rayon::prelude::*;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::index::ngram::BigramDfTable;
use crate::index::reader::IndexReader;
use crate::query::extract::regex_to_query_costed;
use crate::query::plan::NgramQuery;
use crate::search::matcher::{self, SearchConfig};
use crate::util::ig_dir;

#[derive(Deserialize)]
struct QueryRequest {
    /// Project root the query applies to (canonical absolute path). The daemon
    /// uses it to locate the right `TenantState` in its LRU cache.
    #[serde(default)]
    root: String,
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

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize)]
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

/// Single global socket. All clients connect here.
pub fn socket_path() -> PathBuf {
    crate::cache::cache_root().join("daemon.sock")
}

fn pid_path() -> PathBuf {
    crate::cache::cache_root().join("daemon.pid")
}

fn log_path() -> PathBuf {
    crate::cache::cache_root().join("daemon.log")
}

// ─── Tenant state ───────────────────────────────────────────────────────────

struct ReaderView {
    reader: IndexReader,
    df_table: Option<BigramDfTable>,
    last_mtime: SystemTime,
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
        let last_mtime = metadata_mtime(&ig);
        let cap = NonZeroUsize::new(128).unwrap();
        Ok(Self {
            reader_view: RwLock::new(ReaderView {
                reader,
                df_table,
                last_mtime,
            }),
            ig_dir: ig,
            root,
            regex_cache: Mutex::new(LruCache::new(cap)),
            query_cache: Mutex::new(LruCache::new(cap)),
        })
    }

    /// Reload the reader if `metadata.bin` mtime changed since last open.
    /// Cheap (one stat per call) — replaces the per-tenant `notify` watcher.
    fn reload_if_changed(&self) -> bool {
        let current = metadata_mtime(&self.ig_dir);
        let needs = {
            let rv = self.reader_view.read().unwrap_or_else(|e| e.into_inner());
            current != rv.last_mtime
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
                    rv.last_mtime = current;
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

fn metadata_mtime(ig_dir: &Path) -> SystemTime {
    let bin = ig_dir.join("metadata.bin");
    let json = ig_dir.join("metadata.json");
    std::fs::metadata(&bin)
        .or_else(|_| std::fs::metadata(&json))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

// ─── Global state (multi-tenant) ────────────────────────────────────────────

const DEFAULT_MAX_TENANTS: usize = 32;

struct GlobalState {
    tenants: Mutex<LruCache<PathBuf, Arc<TenantState>>>,
}

impl GlobalState {
    fn new(max_tenants: usize) -> Self {
        let cap = NonZeroUsize::new(max_tenants.max(1)).unwrap();
        Self {
            tenants: Mutex::new(LruCache::new(cap)),
        }
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
        let tenant = Arc::new(TenantState::open(&canonical)?);
        {
            let mut guard = self.tenants.lock().unwrap_or_else(|e| e.into_inner());
            guard.put(canonical, Arc::clone(&tenant));
        }
        Ok(tenant)
    }
}

// ─── Server entry points ────────────────────────────────────────────────────

/// Start the global daemon in foreground (this thread blocks accepting
/// connections forever). The path argument is ignored — kept for backwards
/// compatibility with `ig daemon foreground <path>` invocations from old
/// launchd plists.
pub fn start_daemon(_legacy_path: &Path) -> Result<()> {
    purge_legacy_per_project_daemons();

    let max_tenants = std::env::var("IG_DAEMON_TENANTS_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_TENANTS);

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
    }
}

fn process_request(line: &str, state: &GlobalState) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return err_response(format!("invalid request: {}", e)),
    };

    if req.root.is_empty() {
        return err_response("missing 'root' field".into());
    }
    let root = PathBuf::from(&req.root);

    let tenant = match state.tenant_for(&root) {
        Ok(t) => t,
        Err(e) => return err_response(format!("open tenant: {}", e)),
    };

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
            if let Some(ref ft) = req.file_type {
                let ext = rel_path.rsplit('.').next().unwrap_or("");
                if ext != ft.as_str() {
                    return None;
                }
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

    std::fs::write(pid_path(), child.id().to_string()).context("write daemon.pid")?;

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
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        eprintln!("SIGTERM sent to daemon PID {}", pid);
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
        eprintln!(
            "Daemon (global): running (PID {}, socket: {})",
            pid,
            sock.display()
        );
    } else {
        eprintln!("Daemon (global): not running");
    }
    Ok(())
}

fn is_daemon_alive() -> bool {
    if let Ok(pid_str) = std::fs::read_to_string(pid_path())
        && let Ok(pid) = pid_str.trim().parse::<i32>()
    {
        return unsafe { libc::kill(pid, 0) } == 0;
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
    std::fs::write(&plist_path, body).context("write plist")?;

    let status = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .status()
        .context("launchctl load")?;
    if status.success() {
        eprintln!("Installed: {}", plist_path.display());
        eprintln!("Daemon will auto-start on login.");
    } else {
        eprintln!(
            "launchctl load failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall_launchd(_legacy_path: &Path) -> Result<()> {
    let label = "com.ig.daemon.global";
    let plist_path = dirs::home_dir()
        .context("get home dir")?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label));
    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
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

pub fn try_query_daemon(
    root: &Path,
    pattern: &str,
    case_insensitive: bool,
    files_only: bool,
    count_only: bool,
    context: usize,
    file_type: Option<&str>,
) -> Result<Option<DaemonResponse>> {
    let sock = socket_path();
    if !sock.exists() {
        return Ok(None);
    }
    let canonical = root.canonicalize().context("canonicalize root")?;
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

    let mut req = serde_json::json!({
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
    let query =
        match crate::query::extract::regex_to_query(&req.pattern, req.case_insensitive, None) {
            Ok(q) => q,
            Err(e) => {
                return QueryResponse {
                    results: None,
                    error: Some(format!("invalid regex: {}", e)),
                    candidates: 0,
                    total_files,
                    search_ms: 0.0,
                    reloaded,
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
            if let Some(ref ft) = req.file_type {
                let ext = rel_path.rsplit('.').next().unwrap_or("");
                if ext != ft.as_str() {
                    return None;
                }
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
    fn test_metadata_mtime_returns_valid_time() {
        let (dir, root) = setup_test_project();
        let ig = ig_dir(&root);
        let mtime = metadata_mtime(&ig);
        assert_ne!(mtime, SystemTime::UNIX_EPOCH);
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
}
