use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::index::reader::IndexReader;
use crate::query::extract::regex_to_query;
use crate::search::matcher::{self, SearchConfig};
use crate::util::ig_dir;

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

/// Shared state for the daemon — reader + last known mtime of metadata.bin.
struct DaemonState {
    reader: IndexReader,
    ig_dir: PathBuf,
    root: PathBuf,
    last_mtime: SystemTime,
}

impl DaemonState {
    fn new(root: &Path) -> Result<Self> {
        let root = root.to_path_buf();
        let ig = ig_dir(&root);
        let reader = IndexReader::open(&ig).context("open index")?;
        let last_mtime = metadata_mtime(&ig);
        Ok(Self {
            reader,
            ig_dir: ig,
            root,
            last_mtime,
        })
    }

    /// Check if the index has been rebuilt since we last loaded it.
    /// If so, reload the reader. Returns true if reloaded.
    fn reload_if_changed(&mut self) -> bool {
        let current_mtime = metadata_mtime(&self.ig_dir);
        if current_mtime != self.last_mtime {
            match IndexReader::open(&self.ig_dir) {
                Ok(new_reader) => {
                    let old_count = self.reader.metadata.file_count;
                    let new_count = new_reader.metadata.file_count;
                    self.reader = new_reader;
                    self.last_mtime = current_mtime;
                    eprintln!(
                        "Index reloaded: {} → {} files",
                        old_count, new_count
                    );
                    true
                }
                Err(e) => {
                    eprintln!("Failed to reload index: {}", e);
                    false
                }
            }
        } else {
            false
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

    let state = Arc::new(RwLock::new(DaemonState::new(&root)?));
    {
        let s = state.read().unwrap();
        eprintln!(
            "Daemon started: {} files indexed, listening...",
            s.reader.metadata.file_count
        );
    }

    let sock_path = socket_path(&root);
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("bind {}", sock_path.display()))?;

    eprintln!("Socket: {}", sock_path.display());

    let sock_cleanup = sock_path.clone();
    ctrlc_cleanup(sock_cleanup);

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

fn handle_client(
    stream: UnixStream,
    state: &Arc<RwLock<DaemonState>>,
) -> Result<()> {
    let mut buf_reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let mut line = String::new();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }

        // Check for index reload before each query — acquire write lock briefly
        let reloaded = {
            let mut s = state.write().unwrap();
            s.reload_if_changed()
        };

        // Process query with read lock
        let response = {
            let s = state.read().unwrap();
            process_query(&line, &s.reader, &s.root, reloaded)
        };

        let json = serde_json::to_string(&response)?;
        writeln!(writer, "{}", json)?;
        writer.flush()?;
    }

    Ok(())
}

fn process_query(
    line: &str,
    reader: &IndexReader,
    root: &Path,
    reloaded: bool,
) -> QueryResponse {
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
    let total_files = reader.metadata.file_count as usize;

    let query = match regex_to_query(&req.pattern, req.case_insensitive) {
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

    let mut results = Vec::new();

    for doc_id in &candidates {
        let rel_path = reader.file_path(*doc_id);

        if let Some(ref ft) = req.file_type {
            let ext = rel_path.rsplit('.').next().unwrap_or("");
            if ext != ft.as_str() {
                continue;
            }
        }

        match matcher::match_file(root, rel_path, &regex, &config) {
            Ok(Some(file_matches)) => {
                if req.files_only {
                    results.push(MatchResult {
                        file: file_matches.path,
                        line: None,
                        text: None,
                        count: None,
                    });
                } else if req.count_only {
                    results.push(MatchResult {
                        file: file_matches.path,
                        line: None,
                        text: None,
                        count: Some(file_matches.match_count),
                    });
                } else {
                    for m in &file_matches.matches {
                        if m.is_context {
                            continue;
                        }
                        results.push(MatchResult {
                            file: file_matches.path.clone(),
                            line: Some(m.line_number),
                            text: Some(String::from_utf8_lossy(&m.line).to_string()),
                            count: None,
                        });
                    }
                }
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }

    QueryResponse {
        results: Some(results),
        error: None,
        candidates: candidate_count,
        total_files,
        search_ms: start.elapsed().as_secs_f64() * 1000.0,
        reloaded,
    }
}

fn ctrlc_cleanup(sock_path: PathBuf) {
    std::thread::spawn(move || {
        let _ = signal_hook_simple(&sock_path);
    });
}

fn signal_hook_simple(sock_path: &Path) {
    let path = sock_path.to_path_buf();
    let _ = ctrlc::set_handler(move || {
        let _ = std::fs::remove_file(&path);
        std::process::exit(0);
    });
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
        fs::write(src.join("main.rs"), b"fn main() {\n    println!(\"hello\");\n}\n").unwrap();
        fs::write(src.join("lib.rs"), b"pub fn greet() -> String {\n    \"world\".to_string()\n}\n").unwrap();

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
        let mut state = DaemonState::new(&root).unwrap();
        assert!(!state.reload_if_changed(), "should not reload when nothing changed");
        drop(dir);
    }

    #[test]
    fn test_daemon_state_detects_index_rebuild() {
        let (dir, root) = setup_test_project();
        let mut state = DaemonState::new(&root).unwrap();
        let initial_count = state.reader.metadata.file_count;

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
        assert!(
            state.reader.metadata.file_count > initial_count,
            "file count should increase after adding a file: {} vs {}",
            state.reader.metadata.file_count,
            initial_count
        );

        // Second check: no change since we just reloaded
        assert!(!state.reload_if_changed(), "should not reload again immediately");

        drop(dir);
    }

    #[test]
    fn test_process_query_returns_results() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();

        let query_json = r#"{"pattern":"fn main"}"#;
        let response = process_query(query_json, &state.reader, &state.root, false);

        assert!(response.error.is_none(), "should not error");
        let results = response.results.unwrap();
        assert!(!results.is_empty(), "should find 'fn main' in main.rs");
        assert_eq!(results[0].file, "src/main.rs");
        assert!(!response.reloaded);

        drop(dir);
    }

    #[test]
    fn test_process_query_with_reload_flag() {
        let (dir, root) = setup_test_project();
        let state = DaemonState::new(&root).unwrap();

        let query_json = r#"{"pattern":"fn main"}"#;
        let response = process_query(query_json, &state.reader, &state.root, true);

        assert!(response.reloaded, "reloaded flag should be true when passed");

        drop(dir);
    }
}
