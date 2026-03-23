use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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

/// Start the daemon server.
pub fn start_daemon(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;
    let ig = ig_dir(&root);

    let reader = Arc::new(IndexReader::open(&ig).context("open index")?);
    eprintln!(
        "Daemon started: {} files indexed, listening...",
        reader.metadata.file_count
    );

    let sock_path = socket_path(&root);

    // Remove stale socket
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("bind {}", sock_path.display()))?;

    eprintln!("Socket: {}", sock_path.display());

    // Handle SIGINT/SIGTERM to clean up socket
    let sock_cleanup = sock_path.clone();
    ctrlc_cleanup(sock_cleanup);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let reader = Arc::clone(&reader);
                let root = root.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(stream, &reader, &root) {
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

fn handle_client(stream: UnixStream, reader: &IndexReader, root: &Path) -> Result<()> {
    let mut buf_reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let mut line = String::new();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            break; // Client disconnected
        }

        let response = process_query(&line, reader, root);
        let json = serde_json::to_string(&response)?;
        writeln!(writer, "{}", json)?;
        writer.flush()?;
    }

    Ok(())
}

fn process_query(line: &str, reader: &IndexReader, root: &Path) -> QueryResponse {
    let req: QueryRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return QueryResponse {
                results: None,
                error: Some(format!("invalid request: {}", e)),
                candidates: 0,
                total_files: 0,
                search_ms: 0.0,
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

        // Apply type filter
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
    }
}

fn ctrlc_cleanup(sock_path: PathBuf) {
    std::thread::spawn(move || {
        // Simple signal handling: wait for SIGINT
        let _ = signal_hook_simple(&sock_path);
    });
}

fn signal_hook_simple(sock_path: &Path) {
    // Use a simple approach: register a handler that removes the socket
    // This is best-effort — the OS will clean up /tmp eventually anyway
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
