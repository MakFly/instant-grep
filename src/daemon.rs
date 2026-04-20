use std::io::{BufRead, BufReader, Read, Write};
use std::num::NonZeroUsize;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use lru::LruCache;
use notify::{Event, RecursiveMode, Watcher};
use rayon::prelude::*;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::index::ngram::BigramDfTable;
use crate::index::reader::IndexReader;
use crate::index::writer;
use crate::query::extract::regex_to_query;
use crate::query::plan::NgramQuery;
use crate::search::matcher::{self, SearchConfig};
use crate::util::ig_dir;
use crate::walk::DEFAULT_MAX_FILE_SIZE;

#[derive(Deserialize)]
struct QueryRequest {
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

/// Get the socket path for a given root directory.
pub fn socket_path(root: &Path) -> PathBuf {
    let hash = {
        let path_str = root.to_string_lossy();
        let mut h: u64 = 5381;
        for b in path_str.bytes() {
            h = h.wrapping_mul(33).wrapping_add(b as u64);
        }
        h
    };
    PathBuf::from(format!("/tmp/ig-{:x}.sock", hash))
}

/// The read-only view of the index that can be swapped atomically on reload.
struct ReaderView {
    reader: IndexReader,
    df_table: Option<BigramDfTable>,
    last_mtime: SystemTime,
}

/// Shared state for the daemon — reader behind RwLock (readers parallelizable),
/// caches behind individual Mutex so a long-running query only blocks other
/// writers to the same cache, not the reader.
struct DaemonState {
    reader_view: RwLock<ReaderView>,
    ig_dir: PathBuf,
    root: PathBuf,
    regex_cache: Mutex<LruCache<(String, bool), Arc<regex::bytes::Regex>>>,
    query_cache: Mutex<LruCache<(String, bool), NgramQuery>>,
}

impl DaemonState {
    fn new(root: &Path) -> Result<Self> {
        let root = root.to_path_buf();
        let ig = ig_dir(&root);
        let reader = IndexReader::open(&ig).context("open index")?;
        let df_table = if reader.metadata.built_with_idf {
            BigramDfTable::load(&ig)
        } else {
            None
        };
        let last_mtime = metadata_mtime(&ig);
        let cache_cap = NonZeroUsize::new(128).unwrap();
        Ok(Self {
            reader_view: RwLock::new(ReaderView {
                reader,
                df_table,
                last_mtime,
            }),
            ig_dir: ig,
            root,
            regex_cache: Mutex::new(LruCache::new(cache_cap)),
            query_cache: Mutex::new(LruCache::new(cache_cap)),
        })
    }

    /// Check if the index has been rebuilt since we last loaded it.
    /// If so, reload the reader and clear caches. Returns true if reloaded.
    fn reload_if_changed(&self) -> bool {
        let current_mtime = metadata_mtime(&self.ig_dir);
        let needs_reload = {
            let rv = self.reader_view.read().unwrap_or_else(|e| e.into_inner());
            current_mtime != rv.last_mtime
        };
        if !needs_reload {
            return false;
        }
        match IndexReader::open(&self.ig_dir) {
            Ok(new_reader) => {
                let new_df_table = if new_reader.metadata.built_with_idf {
                    BigramDfTable::load(&self.ig_dir)
                } else {
                    None
                };
                let new_count = new_reader.metadata.file_count;
                let old_count = {
                    let mut rv = self.reader_view.write().unwrap_or_else(|e| e.into_inner());
                    let old = rv.reader.metadata.file_count;
                    rv.reader = new_reader;
                    rv.df_table = new_df_table;
                    rv.last_mtime = current_mtime;
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
                eprintln!("Index reloaded: {} → {} files", old_count, new_count);
                true
            }
            Err(e) => {
                eprintln!("Failed to reload index: {}", e);
                false
            }
        }
    }
}

/// Get the mtime of metadata.bin (or metadata.json as fallback).
fn metadata_mtime(ig_dir: &Path) -> SystemTime {
    let bin_path = ig_dir.join("metadata.bin");
    let json_path = ig_dir.join("metadata.json");

    std::fs::metadata(&bin_path)
        .or_else(|_| std::fs::metadata(&json_path))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Start the daemon server.
pub fn start_daemon(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;

    let state = Arc::new(DaemonState::new(&root)?);
    {
        let rv = state.reader_view.read().unwrap_or_else(|e| e.into_inner());
        eprintln!(
            "Daemon started: {} files indexed, listening...",
            rv.reader.metadata.file_count
        );
    }

    // Spawn background thread to warm the OS page cache
    {
        let warm_state = Arc::clone(&state);
        std::thread::spawn(move || {
            warm_page_cache(&warm_state);
        });
    }

    let sock_path = socket_path(&root);
    let _ = std::fs::remove_file(&sock_path);

    // Register cleanup handler BEFORE bind so socket is always cleaned up.
    // Must run synchronously (not in a spawned thread) to avoid a race where
    // SIGTERM arrives before the handler is registered.
    let sock_cleanup = sock_path.clone();
    ctrlc_cleanup(sock_cleanup);

    let listener =
        UnixListener::bind(&sock_path).with_context(|| format!("bind {}", sock_path.display()))?;

    // Restrict socket to owner-only (0o600) so other local users cannot query the index.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0o600 {}", sock_path.display()))?;

    eprintln!("Socket: {}", sock_path.display());

    // Spawn background file watcher thread for automatic index rebuilds
    spawn_file_watcher(&root);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(stream, &state) {
                        eprintln!("Client error: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }

    let _ = std::fs::remove_file(&sock_path);
    Ok(())
}

/// Spawn a background thread that watches the project for file changes and rebuilds the index.
/// Uses the same debounce and filtering logic as `ig watch`.
fn spawn_file_watcher(root: &Path) {
    let root = root.to_path_buf();

    std::thread::spawn(move || {
        if let Err(e) = run_file_watcher(&root) {
            eprintln!("File watcher failed to start: {}", e);
        }
    });
}

/// Run the file watcher loop (blocking). Called from the watcher thread.
fn run_file_watcher(root: &Path) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let dominated_by_ig = event
                .paths
                .iter()
                .all(|p| p.to_string_lossy().contains(".ig/"));
            if !dominated_by_ig {
                let _ = tx.send(event);
            }
        }
    })
    .context("create file watcher")?;

    watcher
        .watch(root, RecursiveMode::Recursive)
        .context("watch directory")?;

    eprintln!("File watcher active: auto-rebuilding index on changes");

    let debounce = Duration::from_millis(500);
    let mut last_rebuild = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(_event) => {
                // Debounce: wait for changes to settle
                while rx.recv_timeout(debounce).is_ok() {}

                if last_rebuild.elapsed() > Duration::from_secs(1) {
                    let start = Instant::now();
                    match writer::build_index(root, true, DEFAULT_MAX_FILE_SIZE) {
                        Ok(meta) => {
                            eprintln!(
                                "Watcher rebuilt: {} files, {} trigrams in {:.0}ms",
                                meta.file_count,
                                meta.ngram_count,
                                start.elapsed().as_secs_f64() * 1000.0,
                            );
                        }
                        Err(e) => {
                            eprintln!("Watcher rebuild error: {}", e);
                        }
                    }
                    last_rebuild = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // No changes, keep watching
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("File watcher channel disconnected");
                break;
            }
        }
    }

    Ok(())
}

fn handle_client(stream: UnixStream, state: &Arc<DaemonState>) -> Result<()> {
    // Set read timeout to prevent hung clients from blocking threads indefinitely
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut buf_reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let mut line = String::new();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }

        // Reload (write lock) is acquired briefly inside reload_if_changed;
        // the actual query runs under a read lock so multiple clients can
        // search in parallel.
        let reloaded = state.reload_if_changed();
        let response = process_query_cached(&line, state, reloaded);

        let json = serde_json::to_string(&response)?;
        writeln!(writer, "{}", json)?;
        writer.flush()?;
    }

    Ok(())
}

/// Process a query using the LRU caches for compiled regexes and NgramQuery results.
fn process_query_cached(line: &str, state: &DaemonState, reloaded: bool) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return QueryResponse {
                results: None,
                error: Some(format!("invalid request: {}", e)),
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
            };
        }
    };

    let start = Instant::now();
    let rv = state.reader_view.read().unwrap_or_else(|e| e.into_inner());
    let total_files = rv.reader.total_file_count() as usize;
    let cache_key = (req.pattern.clone(), req.case_insensitive);

    // NgramQuery: cache lookup or compute
    let query = {
        let cached = state
            .query_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
            .cloned();
        match cached {
            Some(q) => q,
            None => {
                match regex_to_query(&req.pattern, req.case_insensitive, rv.df_table.as_ref()) {
                    Ok(q) => {
                        state
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

    // Compiled regex: cache lookup or compile
    let regex = {
        let cached = state
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
                    state
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

    // Collect candidate paths, applying type filter
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

    let root = &state.root;

    // Parallel regex verification with rayon
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

/// Warm the OS page cache by pre-faulting postings mmap and reading file headers.
fn warm_page_cache(state: &Arc<DaemonState>) {
    let start = Instant::now();
    let rv = state.reader_view.read().unwrap_or_else(|e| e.into_inner());

    // Phase 1: pre-fault postings mmap
    rv.reader.warm_postings();
    let postings_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Phase 2: read first 4KB of each source file (capped at 10,000 files)
    let file_count = rv.reader.metadata.files.len().min(10_000);
    let root = state.root.clone();
    let paths: Vec<String> = rv.reader.metadata.files[..file_count]
        .iter()
        .map(|f| f.path.clone())
        .collect();
    drop(rv); // release lock during I/O

    let mut warmed = 0usize;
    let mut buf = [0u8; 4096];
    for path in &paths {
        let full = root.join(path);
        if let Ok(mut f) = std::fs::File::open(&full) {
            let _ = f.read(&mut buf);
            warmed += 1;
        }
    }

    eprintln!(
        "Page cache warmed: postings in {:.1}ms, {} files prefetched in {:.0}ms",
        postings_ms,
        warmed,
        start.elapsed().as_secs_f64() * 1000.0,
    );
}

#[cfg(test)]
fn process_query(line: &str, reader: &IndexReader, root: &Path, reloaded: bool) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return QueryResponse {
                results: None,
                error: Some(format!("invalid request: {}", e)),
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
                reloaded: false,
            };
        }
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

    // Collect candidate paths, applying type filter
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

    // Parallel regex verification with rayon
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

/// Install the SIGINT/SIGTERM cleanup handler synchronously so no signal can
/// arrive between start_daemon() and handler registration.
fn ctrlc_cleanup(sock_path: PathBuf) {
    let _ = ctrlc::set_handler(move || {
        let _ = std::fs::remove_file(&sock_path);
        std::process::exit(0);
    });
}

/// PID file path within .ig/
fn pid_path(ig_dir: &Path) -> PathBuf {
    ig_dir.join("daemon.pid")
}

/// Start the daemon in the background by re-executing ourselves.
pub fn start_daemon_background(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = crate::util::ig_dir(&root);

    // Check if already running based on the PID file alone; sock.exists()
    // is a TOCTOU signal since another process could create/remove it between
    // the check and a subsequent bind.
    if is_daemon_alive(&ig) {
        eprintln!("Daemon is already running");
        return Ok(());
    }

    let exe = std::env::current_exe().context("get current exe")?;
    let log_path = ig.join("daemon.log");

    // Re-launch ourselves with IG_DAEMON_FOREGROUND=1
    let log_file = std::fs::File::create(&log_path).context("create daemon.log")?;
    let log_err = log_file.try_clone()?;

    let child = std::process::Command::new(&exe)
        .args(["daemon", "foreground", &root.to_string_lossy()])
        .env("IG_DAEMON_FOREGROUND", "1")
        .stdout(log_file)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()
        .context("spawn daemon process")?;

    // Write PID
    let pid = child.id();
    std::fs::write(pid_path(&ig), pid.to_string()).context("write daemon.pid")?;

    eprintln!("Daemon started (PID {}), log: {}", pid, log_path.display());
    Ok(())
}

/// Stop a running daemon.
pub fn stop_daemon(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = crate::util::ig_dir(&root);
    let sock = socket_path(&root);
    let pid_file = pid_path(&ig);

    if pid_file.exists() {
        if let Ok(pid_str) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = pid_str.trim().parse::<i32>()
        {
            // Send SIGTERM
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            eprintln!("Sent SIGTERM to daemon (PID {})", pid);
        }
        let _ = std::fs::remove_file(&pid_file);
    }

    // Also clean up socket
    let _ = std::fs::remove_file(&sock);

    Ok(())
}

/// Show daemon status.
pub fn daemon_status(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = crate::util::ig_dir(&root);
    let sock = socket_path(&root);

    if sock.exists() && is_daemon_alive(&ig) {
        let pid = std::fs::read_to_string(pid_path(&ig))
            .unwrap_or_default()
            .trim()
            .to_string();
        eprintln!("Daemon: running (PID {}, socket: {})", pid, sock.display());
    } else {
        eprintln!("Daemon: not running");
    }

    Ok(())
}

/// Check if the daemon process is alive via its PID file.
fn is_daemon_alive(ig_dir: &Path) -> bool {
    let pid_file = pid_path(ig_dir);
    if let Ok(pid_str) = std::fs::read_to_string(pid_file)
        && let Ok(pid) = pid_str.trim().parse::<i32>()
    {
        // kill(pid, 0) checks if process exists without sending a signal
        return unsafe { libc::kill(pid, 0) } == 0;
    }
    false
}

/// Generate a launchd plist label for a project.
fn launchd_label(root: &Path) -> String {
    let hash = {
        let path_str = root.to_string_lossy();
        let mut h: u64 = 5381;
        for b in path_str.bytes() {
            h = h.wrapping_mul(33).wrapping_add(b as u64);
        }
        h
    };
    format!("com.ig.daemon.{:x}", hash)
}

/// Install a launchd plist for auto-restart on boot.
pub fn install_launchd(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = crate::util::ig_dir(&root);
    let exe = std::env::current_exe().context("get current exe")?;
    let label = launchd_label(&root);

    let plist_dir = dirs::home_dir()
        .context("get home dir")?
        .join("Library/LaunchAgents");
    std::fs::create_dir_all(&plist_dir).context("create LaunchAgents dir")?;

    let plist_path = plist_dir.join(format!("{}.plist", label));

    let log_path = ig.join("daemon.log");

    let plist_content = format!(
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
        <string>{root}</string>
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
    <key>WorkingDirectory</key>
    <string>{root}</string>
</dict>
</plist>"#,
        label = label,
        exe = exe.display(),
        root = root.display(),
        log = log_path.display(),
    );

    std::fs::write(&plist_path, &plist_content).context("write plist")?;

    // Load the service
    let status = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .status()
        .context("launchctl load")?;

    if status.success() {
        eprintln!("Installed: {}", plist_path.display());
        eprintln!("Label: {}", label);
        eprintln!("Daemon will auto-start on boot and restart on crash");
    } else {
        eprintln!(
            "launchctl load failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

/// Uninstall the launchd plist.
pub fn uninstall_launchd(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let label = launchd_label(&root);

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
        eprintln!("No plist found for this project");
    }

    // Also stop the daemon if running
    stop_daemon(&root)?;

    Ok(())
}

/// Send a query to a running daemon and return the response.
pub fn query_daemon(root: &Path, pattern: &str, case_insensitive: bool) -> Result<String> {
    let root = root.canonicalize().context("canonicalize root")?;
    let sock = socket_path(&root);

    let stream = UnixStream::connect(&sock)
        .with_context(|| format!("connect to daemon at {}", sock.display()))?;

    let req = serde_json::json!({
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    /// Helper: create a minimal project with files, build an index, return the temp dir path.
    fn setup_test_project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create some source files
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

        // Build index
        crate::index::writer::build_index(&root, false, 1_048_576).unwrap();

        (dir, root)
    }

    #[test]
    fn test_metadata_mtime_returns_valid_time() {
        let (dir, root) = setup_test_project();
        let ig = ig_dir(&root);
        let mtime = metadata_mtime(&ig);
        assert_ne!(mtime, SystemTime::UNIX_EPOCH, "mtime should not be epoch");
        drop(dir);
    }

    #[test]
    fn test_daemon_state_detects_no_change() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();
        assert!(
            !state.reload_if_changed(),
            "should not reload when nothing changed"
        );
        drop(dir);
    }

    #[test]
    fn test_daemon_state_detects_index_rebuild() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();
        let initial_count = state.reader_view.read().unwrap().reader.metadata.file_count;

        // Add a new file
        let src = root.join("src");
        fs::write(src.join("new_file.rs"), b"pub fn new_func() { todo!() }\n").unwrap();

        // Delete existing index to force a full rebuild (not incremental skip)
        let ig = ig_dir(&root);
        let _ = fs::remove_dir_all(&ig);

        // Small sleep to ensure mtime differs (filesystem granularity)
        std::thread::sleep(Duration::from_millis(50));
        crate::index::writer::build_index(&root, false, 1_048_576).unwrap();

        // Now the daemon should detect the change
        let reloaded = state.reload_if_changed();
        assert!(reloaded, "should detect index was rebuilt");
        let new_count = state.reader_view.read().unwrap().reader.metadata.file_count;
        assert!(
            new_count > initial_count,
            "file count should increase after adding a file: {} vs {}",
            new_count,
            initial_count
        );

        // Second check: no change since we just reloaded
        assert!(
            !state.reload_if_changed(),
            "should not reload again immediately"
        );

        drop(dir);
    }

    #[test]
    fn test_process_query_returns_results() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();
        let rv = state.reader_view.read().unwrap();

        let query_json = r#"{"pattern":"fn main"}"#;
        let response = process_query(query_json, &rv.reader, &state.root, false);

        assert!(response.error.is_none(), "should not error");
        let results = response.results.unwrap();
        assert!(!results.is_empty(), "should find 'fn main' in main.rs");
        assert_eq!(results[0].file, "src/main.rs");
        assert!(!response.reloaded);

        drop(rv);
        drop(dir);
    }

    #[test]
    fn test_process_query_with_reload_flag() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();
        let rv = state.reader_view.read().unwrap();

        let query_json = r#"{"pattern":"fn main"}"#;
        let response = process_query(query_json, &rv.reader, &state.root, true);

        assert!(
            response.reloaded,
            "reloaded flag should be true when passed"
        );

        drop(rv);
        drop(dir);
    }
}
