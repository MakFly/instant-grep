//! Minimal OpenAI embeddings client. POST /v1/embeddings, blocking, via ureq (already in deps).

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::json;

const ENDPOINT: &str = "https://api.openai.com/v1/embeddings";

#[derive(Debug)]
pub struct EmbedResponse {
    pub embedding: Vec<f32>,
    pub prompt_tokens: u32,
}

#[derive(Deserialize)]
struct ApiResponse {
    data: Vec<ApiDatum>,
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct ApiDatum {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: u32,
}

/// Single-input embedding (Phase 1). Wraps `embed_batch` for convenience.
pub fn embed_one(api_key: &str, model: &str, text: &str) -> Result<EmbedResponse> {
    let mut batch = embed_batch(api_key, model, &[text.to_string()])?;
    let v = batch.embeddings.pop().ok_or_else(|| anyhow!("empty"))?;
    Ok(EmbedResponse {
        embedding: v,
        prompt_tokens: batch.prompt_tokens,
    })
}

#[derive(Debug)]
pub struct BatchResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub prompt_tokens: u32,
}

/// Batch up to ~100 inputs per call (OpenAI hard limit is 2048; 100 is conservative).
pub fn embed_batch(api_key: &str, model: &str, inputs: &[String]) -> Result<BatchResponse> {
    let body = json!({
        "model": model,
        "input": inputs,
        "encoding_format": "float",
    });

    let resp = ureq::post(ENDPOINT)
        .header("Authorization", &format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .with_context(|| "POST /v1/embeddings failed")?;

    let status = resp.status();
    let mut resp = resp;
    let body_str = resp
        .body_mut()
        .read_to_string()
        .with_context(|| "read response body")?;

    if !status.is_success() {
        return Err(anyhow!(
            "OpenAI API error (HTTP {}): {}",
            status.as_u16(),
            body_str
        ));
    }

    let parsed: ApiResponse =
        serde_json::from_str(&body_str).with_context(|| "parse embeddings response JSON")?;

    if parsed.data.len() != inputs.len() {
        return Err(anyhow!(
            "OpenAI returned {} embeddings for {} inputs",
            parsed.data.len(),
            inputs.len()
        ));
    }

    Ok(BatchResponse {
        embeddings: parsed.data.into_iter().map(|d| d.embedding).collect(),
        prompt_tokens: parsed.usage.prompt_tokens,
    })
}

/// Cost in USD for a given (model, prompt_tokens). Public posted prices, April 2026.
/// Reconfirm in https://openai.com/api/pricing — these constants are documentation.
pub fn estimate_cost(model: &str, tokens: u32) -> f64 {
    let per_million = match model {
        "text-embedding-3-small" => 0.02_f64,
        "text-embedding-3-large" => 0.13_f64,
        "text-embedding-ada-002" => 0.10_f64,
        _ => 0.02_f64,
    };
    (tokens as f64) * per_million / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_small_one_token() {
        let c = estimate_cost("text-embedding-3-small", 1);
        assert!((c - 0.00000002).abs() < 1e-12);
    }

    #[test]
    fn cost_large_one_million() {
        let c = estimate_cost("text-embedding-3-large", 1_000_000);
        assert!((c - 0.13).abs() < 1e-9);
    }

    #[test]
    fn cost_unknown_model_fallback_small() {
        let c = estimate_cost("does-not-exist", 1_000_000);
        assert!((c - 0.02).abs() < 1e-9);
    }
}
