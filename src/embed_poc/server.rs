//! Phase 3 — `ig embed-poc serve`: tiny_http JSON API + (optional) static SPA.
//! Sync, blocking, one thread per request. POC only — no auth, 127.0.0.1 bind.

use crate::embed_poc::{config, openai, search, store};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    #[serde(default = "default_top")]
    top: usize,
}

fn default_top() -> usize {
    5
}

pub fn run_serve(port: u16, ui_dir: Option<String>) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(&addr).map_err(|e| anyhow!("bind {}: {}", addr, e))?;
    let server = Arc::new(server);

    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ui_dir = ui_dir.map(PathBuf::from);

    eprintln!("ig embed-poc serve");
    eprintln!("  → http://{}", addr);
    if let Some(p) = &ui_dir {
        eprintln!("  → static SPA: {}", p.display());
    } else {
        eprintln!("  → static SPA: (none — JSON API only)");
    }
    eprintln!("  → store: {}", root.join(store::STORE_PATH).display());
    eprintln!("  Ctrl-C to stop.");

    let n_workers = 4;
    let mut handles = Vec::new();
    for _ in 0..n_workers {
        let server = Arc::clone(&server);
        let root = root.clone();
        let ui_dir = ui_dir.clone();
        handles.push(thread::spawn(move || {
            for req in server.incoming_requests() {
                if let Err(e) = handle(req, &root, ui_dir.as_deref()) {
                    eprintln!("[handler] {:#}", e);
                }
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

fn handle(mut req: Request, root: &Path, ui_dir: Option<&Path>) -> Result<()> {
    let started = Instant::now();
    let method = req.method().clone();
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();

    let result: std::result::Result<Response<std::io::Cursor<Vec<u8>>>, (u16, String)> =
        match (&method, path.as_str()) {
            (Method::Get, "/api/status") => Ok(json_ok(api_status(root))),
            (Method::Post, "/api/search") => {
                let mut body = String::new();
                match req.as_reader().read_to_string(&mut body) {
                    Ok(_) => match serde_json::from_str::<SearchBody>(&body) {
                        Ok(parsed) => api_search(root, &parsed.query, parsed.top)
                            .map(json_ok)
                            .map_err(|e| (500, format!("{:#}", e))),
                        Err(e) => Err((400, format!("bad json: {}", e))),
                    },
                    Err(e) => Err((400, format!("read body: {}", e))),
                }
            }
            (Method::Get, "/api/chunks") => {
                let limit = parse_query_param(&url, "limit").unwrap_or(50);
                api_chunks(root, limit)
                    .map(json_ok)
                    .map_err(|e| (500, format!("{:#}", e)))
            }
            (Method::Get, p) => match ui_dir {
                Some(dir) => serve_static(dir, p).map_err(|e| (500, format!("{:#}", e))),
                None => Ok(landing_page()),
            },
            _ => Err((404, format!("no route for {} {}", method.as_str(), path))),
        };

    let ms = started.elapsed().as_millis();
    match result {
        Ok(resp) => {
            eprintln!("{} {} → 200 ({} ms)", method.as_str(), path, ms);
            req.respond(resp).context("respond")?;
        }
        Err((code, msg)) => {
            eprintln!("{} {} → {} ({} ms) {}", method.as_str(), path, code, ms, msg);
            let body = json!({ "error": msg }).to_string();
            req.respond(json_response(code, body)).context("respond")?;
        }
    }
    Ok(())
}

fn api_status(root: &Path) -> Value {
    match store::Store::load(root) {
        Ok(Some(s)) => json!({
            "ready": true,
            "version": s.version,
            "provider": s.provider,
            "model": s.model,
            "dim": s.dim,
            "chunks": s.chunks.len(),
            "total_tokens": s.total_tokens,
            "total_cost_usd": s.total_cost_usd,
            "store_path": root.join(store::STORE_PATH).to_string_lossy(),
        }),
        Ok(None) => json!({
            "ready": false,
            "hint": "run `ig embed-poc index <dir>` first",
        }),
        Err(e) => json!({ "ready": false, "error": format!("{:#}", e) }),
    }
}

fn api_search(root: &Path, query: &str, top: usize) -> Result<Value> {
    let cfg = config::load()?;
    let store = store::Store::load(root)?
        .ok_or_else(|| anyhow!("no store yet — run `ig embed-poc index <dir>` first"))?;

    let t0 = Instant::now();
    let qresp = openai::embed_one(&cfg.openai_api_key, &store.model, query)?;
    let api_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let hits = search::search(&store, &qresp.embedding, top);
    let cosine_ms = t1.elapsed().as_millis();

    let hits_json: Vec<Value> = hits
        .iter()
        .map(|h| {
            let preview = read_preview(root, &h.chunk.file, h.chunk.start_line, h.chunk.end_line);
            json!({
                "score": h.score,
                "file": h.chunk.file,
                "start_line": h.chunk.start_line,
                "end_line": h.chunk.end_line,
                "tokens": h.chunk.tokens,
                "preview": preview,
            })
        })
        .collect();

    Ok(json!({
        "query": query,
        "query_tokens": qresp.prompt_tokens,
        "query_cost_usd": openai::estimate_cost(&store.model, qresp.prompt_tokens),
        "openai_ms": api_ms,
        "cosine_ms": cosine_ms,
        "scanned": store.chunks.len(),
        "hits": hits_json,
    }))
}

fn api_chunks(root: &Path, limit: usize) -> Result<Value> {
    let s = store::Store::load(root)?
        .ok_or_else(|| anyhow!("no store yet — run `ig embed-poc index <dir>` first"))?;
    let chunks: Vec<Value> = s
        .chunks
        .iter()
        .take(limit)
        .map(|c| {
            json!({
                "id": c.id,
                "file": c.file,
                "start_line": c.start_line,
                "end_line": c.end_line,
                "tokens": c.tokens,
                "embedding": c.embedding,
            })
        })
        .collect();
    Ok(json!({
        "total": s.chunks.len(),
        "returned": chunks.len(),
        "dim": s.dim,
        "chunks": chunks,
    }))
}

fn read_preview(root: &Path, rel: &str, start: usize, end: usize) -> Vec<String> {
    let path = root.join(rel);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let s = start.saturating_sub(1);
    let e = end.min(lines.len());
    lines[s..e]
        .iter()
        .take(8)
        .map(|l| l.chars().take(160).collect::<String>())
        .collect()
}

fn parse_query_param(url: &str, key: &str) -> Option<usize> {
    let q = url.split_once('?')?.1;
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=')
            && k == key
        {
            return v.parse().ok();
        }
    }
    None
}

fn json_ok(v: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(200, v.to_string())
}

fn json_response(code: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(body.into_bytes())
        .with_status_code(StatusCode(code))
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
        .with_header(
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        )
}

fn serve_static(
    dir: &Path,
    req_path: &str,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let rel = req_path.trim_start_matches('/');
    let mut candidate = if rel.is_empty() {
        dir.join("index.html")
    } else {
        dir.join(rel)
    };
    if candidate.is_dir() {
        candidate = candidate.join("index.html");
    }
    // SPA fallback: missing file → serve index.html
    if !candidate.exists() {
        candidate = dir.join("index.html");
    }
    let bytes = std::fs::read(&candidate)
        .with_context(|| format!("read {}", candidate.display()))?;
    let mime = mime_for(&candidate);
    Ok(Response::from_data(bytes)
        .with_header(Header::from_bytes(&b"Content-Type"[..], mime.as_bytes()).unwrap()))
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("map") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn landing_page() -> Response<std::io::Cursor<Vec<u8>>> {
    let body = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>ig embed-poc</title>
<style>
  body { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; max-width: 760px;
         margin: 4rem auto; padding: 0 1.5rem; color: #ddd; background: #111; }
  h1 { color: #6cf; } code { background: #222; padding: 2px 6px; border-radius: 4px; }
  pre { background: #1a1a1a; padding: 1rem; border-radius: 6px; overflow-x: auto; }
  a { color: #6cf; }
</style></head><body>
<h1>ig embed-poc — JSON API</h1>
<p>The SPA isn't built yet. Available endpoints:</p>
<pre>GET  /api/status
POST /api/search  { "query": "...", "top": 5 }
GET  /api/chunks?limit=50</pre>
<p>To attach a UI, build the SPA in <code>ui/dist/</code> and start with:</p>
<pre>ig embed-poc serve --ui ui/dist</pre>
</body></html>"#;
    Response::from_data(body.as_bytes().to_vec())
        .with_status_code(StatusCode(200))
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
        )
}
