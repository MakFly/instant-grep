//! POC — OpenAI embeddings + retrieval.
//!
//! Throwaway code on branch `feat/ui-embeddings` to learn how embeddings work
//! before committing to the full plan in `docs/plans/PLAN-ui-embeddings.md`.
//! See `~/.claude/plans/distributed-inventing-crab.md` for the phased plan.

pub mod chunk;
pub mod config;
pub mod openai;
pub mod search;
pub mod store;

use crate::walk;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const BATCH_SIZE: usize = 64;

fn project_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn is_text_file(path: &Path) -> bool {
    // Heuristic: read first 512 bytes, reject if NUL byte present.
    match fs::File::open(path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = [0u8; 512];
            let n = f.read(&mut buf).unwrap_or(0);
            !buf[..n].contains(&0)
        }
        Err(_) => false,
    }
}

/// Phase 2 — `ig embed-poc index <dir>`: chunk + batch-embed + store JSON.
pub fn run_index(dir: Option<String>, yes: bool) -> Result<()> {
    let cfg = config::load()?;
    let root = project_root();
    let target = dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let target = if target.is_absolute() {
        target
    } else {
        root.join(&target)
    };

    eprintln!("Walking {} ...", target.display());
    let files = walk::walk_files(&target, true, walk::DEFAULT_MAX_FILE_SIZE, None, None)?;

    let mut all_chunks: Vec<chunk::RawChunk> = Vec::new();
    for path in &files {
        if !is_text_file(path) {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let rel = path.strip_prefix(&root).unwrap_or(path);
        for c in chunk::chunk_file(rel, &content) {
            all_chunks.push(c);
        }
    }

    let total_chunks = all_chunks.len();
    let total_tokens_est: u64 = all_chunks
        .iter()
        .map(|c| chunk::estimate_tokens(&c.text) as u64)
        .sum();
    let est_cost = openai::estimate_cost(&cfg.embed_model, total_tokens_est as u32);

    eprintln!(
        "Plan: {} files → {} chunks · ~{} tokens · est. ${:.4} ({} model)",
        files.len(),
        total_chunks,
        total_tokens_est,
        est_cost,
        cfg.embed_model
    );

    if total_chunks == 0 {
        return Err(anyhow!("nothing to embed (empty dir or all binary)"));
    }

    if !yes {
        eprint!("Proceed? [y/N] ");
        io::stderr().flush().ok();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    // Probe dim with first batch
    let started = Instant::now();
    let mut store_opt: Option<store::Store> = None;
    let mut id: u32 = 0;
    let mut total_tokens: u64 = 0;
    let mut total_cost: f64 = 0.0;

    for (batch_idx, batch) in all_chunks.chunks(BATCH_SIZE).enumerate() {
        let inputs: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
        let resp = openai::embed_batch(&cfg.openai_api_key, &cfg.embed_model, &inputs)
            .with_context(|| format!("batch {}", batch_idx))?;

        let dim = resp.embeddings.first().map(|v| v.len()).unwrap_or(0);
        let store =
            store_opt.get_or_insert_with(|| store::Store::new("openai", &cfg.embed_model, dim));

        for (raw, emb) in batch.iter().zip(resp.embeddings.into_iter()) {
            store.chunks.push(store::StoredChunk {
                id,
                file: raw.file.to_string_lossy().to_string(),
                start_line: raw.start_line,
                end_line: raw.end_line,
                tokens: chunk::estimate_tokens(&raw.text),
                embedding: emb,
            });
            id += 1;
        }

        total_tokens += resp.prompt_tokens as u64;
        total_cost += openai::estimate_cost(&cfg.embed_model, resp.prompt_tokens);

        eprintln!(
            "  batch {}/{} · {} chunks · {} tokens · ${:.6}",
            batch_idx + 1,
            total_chunks.div_ceil(BATCH_SIZE),
            inputs.len(),
            resp.prompt_tokens,
            openai::estimate_cost(&cfg.embed_model, resp.prompt_tokens)
        );
    }

    let mut store = store_opt.ok_or_else(|| anyhow!("no batches processed"))?;
    store.total_tokens = total_tokens;
    store.total_cost_usd = total_cost;
    let path = store.save(&root)?;

    eprintln!(
        "\n✓ Indexed {} chunks · dim {} · {} tokens · ${:.4} · {:.1}s",
        store.chunks.len(),
        store.dim,
        total_tokens,
        total_cost,
        started.elapsed().as_secs_f64()
    );
    eprintln!("  Wrote {}", path.display());
    Ok(())
}

/// Phase 2 — `ig embed-poc inspect`: human-readable summary of the store.
pub fn run_inspect(limit: usize) -> Result<()> {
    let root = project_root();
    let store = store::Store::load(&root)?
        .ok_or_else(|| anyhow!("no store yet — run `ig embed-poc index <dir>` first"))?;

    println!("Store        : {}", root.join(store::STORE_PATH).display());
    println!("Version      : {}", store.version);
    println!("Provider     : {}", store.provider);
    println!("Model        : {}", store.model);
    println!("Dim          : {}", store.dim);
    println!("Chunks       : {}", store.chunks.len());
    println!("Total tokens : {}", store.total_tokens);
    println!("Total cost   : ${:.6}", store.total_cost_usd);
    println!();
    println!("First {} chunks:", limit.min(store.chunks.len()));
    for c in store.chunks.iter().take(limit) {
        let preview: Vec<String> = c
            .embedding
            .iter()
            .take(5)
            .map(|v| format!("{:+.4}", v))
            .collect();
        println!(
            "  #{:<4} {} L{}-{} · {} tokens · [{}, …]",
            c.id,
            c.file,
            c.start_line,
            c.end_line,
            c.tokens,
            preview.join(", ")
        );
    }
    Ok(())
}

/// Phase 2 — `ig embed-poc search "<query>"`: cosine top-N over the store.
pub fn run_search(query: &str, top_n: usize) -> Result<()> {
    let cfg = config::load()?;
    let root = project_root();
    let store = store::Store::load(&root)?
        .ok_or_else(|| anyhow!("no store yet — run `ig embed-poc index <dir>` first"))?;

    let started = Instant::now();
    let qresp = openai::embed_one(&cfg.openai_api_key, &store.model, query)?;
    let api_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let hits = search::search(&store, &qresp.embedding, top_n);
    let local_ms = started.elapsed().as_millis();

    println!(
        "Query: {:?}  ({} tokens, ${:.8})",
        query,
        qresp.prompt_tokens,
        openai::estimate_cost(&store.model, qresp.prompt_tokens)
    );
    println!(
        "Latency: OpenAI {} ms · local cosine {} ms · {} chunks scanned",
        api_ms,
        local_ms,
        store.chunks.len()
    );
    println!();

    for (rank, hit) in hits.iter().enumerate() {
        println!(
            "#{}  score {:.4}  {}:{}-{}",
            rank + 1,
            hit.score,
            hit.chunk.file,
            hit.chunk.start_line,
            hit.chunk.end_line
        );
        if let Ok(content) = fs::read_to_string(root.join(&hit.chunk.file)) {
            let lines: Vec<&str> = content.lines().collect();
            let s = hit.chunk.start_line.saturating_sub(1);
            let e = hit.chunk.end_line.min(lines.len());
            let preview: Vec<&str> = lines[s..e].iter().take(4).copied().collect();
            for l in preview {
                let truncated: String = l.chars().take(120).collect();
                println!("    | {}", truncated);
            }
            if e - s > 4 {
                println!("    | … ({} more lines)", e - s - 4);
            }
        }
        println!();
    }
    Ok(())
}

/// Phase 1 — `ig embed-poc hello <text>`: one input, one vector, console summary.
pub fn run_hello(text: &str) -> Result<()> {
    let cfg = config::load()?;
    let resp = openai::embed_one(&cfg.openai_api_key, &cfg.embed_model, text)?;

    let dim = resp.embedding.len();
    let l2 = (resp.embedding.iter().map(|v| v * v).sum::<f32>()).sqrt();
    let first10: Vec<String> = resp
        .embedding
        .iter()
        .take(10)
        .map(|v| format!("{:+.4}", v))
        .collect();
    let cost = openai::estimate_cost(&cfg.embed_model, resp.prompt_tokens);

    println!("Provider     : openai");
    println!("Model        : {}", cfg.embed_model);
    println!("Input tokens : {}", resp.prompt_tokens);
    println!("Cost         : ${:.10}", cost);
    println!("Vector dim   : {}", dim);
    println!("First 10     : [{}]", first10.join(", "));
    println!(
        "L2 norm      : {:.4}  (OpenAI vectors are L2-normalised → cosine = dot product)",
        l2
    );
    Ok(())
}
