//! POC — OpenAI embeddings + retrieval.
//!
//! Throwaway code on branch `feat/ui-embeddings` to learn how embeddings work
//! before committing to the full plan in `docs/plans/PLAN-ui-embeddings.md`.
//! See `~/.claude/plans/distributed-inventing-crab.md` for the phased plan.

pub mod config;
pub mod openai;

use anyhow::Result;

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
