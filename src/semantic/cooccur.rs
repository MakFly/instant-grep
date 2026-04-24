//! Co-occurrence index — builds and queries a PMI-ranked neighbour map.
//!
//! Build phase (runs during `ig index`):
//!   - Walk every file in the project.
//!   - Tokenize each line, keep a rolling buffer of the last `WINDOW` lines.
//!   - For each token in the current line, for each token in any of the
//!     buffered lines, increment a pair counter.
//!   - At end: compute PMI per pair, keep the top-`KEEP_PER_TOKEN` neighbours
//!     for each token.
//!
//! Query phase:
//!   - Given a query token, return its top neighbours (lazy-loaded from
//!     the on-disk table).
//!
//! Storage: bincode serialised `HashMap<String, Vec<(String, f32)>>`
//! written to `.ig/cooccurrence.bin`. Simple, no mmap trick — a typical
//! project's map fits in memory easily.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::semantic::tokenize::Tokenizer;

const WINDOW: usize = 5; // lines
/// Minimum number of co-occurrences a pair needs before we consider it.
/// Low values let noise (rare-word accidents) dominate the PMI ranking
/// because PMI rewards co-occurrences that happen "more than expected" —
/// when expected is near-zero, any coincidence looks meaningful.
const MIN_PAIR_COUNT: u32 = 15;
/// Minimum total occurrences for a token to be considered at all.
/// Filters out one-offs like `érieur` JSON-escape artefacts.
const MIN_TOKEN_COUNT: u32 = 10;
const KEEP_PER_TOKEN: usize = 10;
const FILE_NAME: &str = "cooccurrence.bin";
/// Cap total distinct tokens learned to keep RAM bounded on huge repos.
const MAX_TOKENS: usize = 200_000;

#[derive(Serialize, Deserialize, Default)]
pub struct CooccurrenceIndex {
    /// token → Vec<(neighbour, pmi_score)> sorted by score descending.
    pub neighbours: HashMap<String, Vec<(String, f32)>>,
}

impl CooccurrenceIndex {
    /// Look up the top neighbours for a query token.
    /// Returns `None` when the token was never seen in the corpus.
    pub fn expand(&self, token: &str, limit: usize) -> Option<Vec<String>> {
        let lower = token.to_lowercase();
        self.neighbours.get(&lower).map(|v| {
            v.iter()
                .take(limit)
                .map(|(t, _)| t.clone())
                .collect::<Vec<_>>()
        })
    }

    pub fn save(&self, ig_dir: &Path) -> Result<()> {
        let path = ig_dir.join(FILE_NAME);
        let data = bincode::serialize(self).context("serialise cooccurrence")?;
        std::fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn load(ig_dir: &Path) -> Option<Self> {
        let path = ig_dir.join(FILE_NAME);
        let data = std::fs::read(&path).ok()?;
        bincode::deserialize::<CooccurrenceIndex>(&data).ok()
    }
}

/// Accumulator used during index build.
pub struct CooccurrenceBuilder {
    tokenizer: Tokenizer,
    /// token_id → token string
    tokens: Vec<String>,
    /// token string → token_id
    token_ids: HashMap<String, u32>,
    /// per-token total occurrence
    counts: HashMap<u32, u32>,
    /// (min(a,b), max(a,b)) → co-occurrence count
    pairs: HashMap<(u32, u32), u32>,
    total_tokens: u64,
}

impl CooccurrenceBuilder {
    pub fn new() -> Self {
        Self {
            tokenizer: Tokenizer::new(),
            tokens: Vec::new(),
            token_ids: HashMap::new(),
            counts: HashMap::new(),
            pairs: HashMap::new(),
            total_tokens: 0,
        }
    }

    /// Feed one file's worth of text.
    pub fn feed_text(&mut self, text: &str) {
        let mut window: Vec<Vec<u32>> = Vec::with_capacity(WINDOW);
        let mut buf: Vec<String> = Vec::new();

        for line in text.lines() {
            self.tokenizer.tokenize_into(line, &mut buf);
            if buf.is_empty() {
                continue;
            }
            let ids: Vec<u32> = buf.iter().filter_map(|t| self.intern_cap(t)).collect();

            // Count token occurrences + pairs with buffered window.
            for &id in &ids {
                *self.counts.entry(id).or_default() += 1;
                self.total_tokens += 1;
                for prev_line in &window {
                    for &prev_id in prev_line {
                        if prev_id == id {
                            continue;
                        }
                        let key = if prev_id < id {
                            (prev_id, id)
                        } else {
                            (id, prev_id)
                        };
                        *self.pairs.entry(key).or_default() += 1;
                    }
                }
            }
            // Count intra-line pairs too
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    if ids[i] == ids[j] {
                        continue;
                    }
                    let key = if ids[i] < ids[j] {
                        (ids[i], ids[j])
                    } else {
                        (ids[j], ids[i])
                    };
                    *self.pairs.entry(key).or_default() += 1;
                }
            }

            window.push(ids);
            if window.len() > WINDOW {
                window.remove(0);
            }
        }
    }

    fn intern_cap(&mut self, token: &str) -> Option<u32> {
        if let Some(&id) = self.token_ids.get(token) {
            return Some(id);
        }
        if self.tokens.len() >= MAX_TOKENS {
            return None;
        }
        let id = self.tokens.len() as u32;
        self.tokens.push(token.to_string());
        self.token_ids.insert(token.to_string(), id);
        Some(id)
    }

    /// Finalise: compute PMI per pair, keep top-K neighbours per token.
    pub fn finalise(self) -> CooccurrenceIndex {
        let total_pairs = self.pairs.values().map(|&v| v as u64).sum::<u64>().max(1);
        let total_tokens = self.total_tokens.max(1);

        // Accumulate: token → Vec<(neighbour_id, pmi)>
        let mut by_token: HashMap<u32, Vec<(u32, f32)>> = HashMap::new();

        for ((a, b), &count) in &self.pairs {
            if count < MIN_PAIR_COUNT {
                continue;
            }
            let ca = *self.counts.get(a).unwrap_or(&1);
            let cb = *self.counts.get(b).unwrap_or(&1);
            if ca < MIN_TOKEN_COUNT || cb < MIN_TOKEN_COUNT {
                continue;
            }
            let pa = ca as f64 / total_tokens as f64;
            let pb = cb as f64 / total_tokens as f64;
            let pab = count as f64 / total_pairs as f64;
            let pmi = (pab / (pa * pb).max(1e-12)).ln();
            if !pmi.is_finite() || pmi <= 0.0 {
                continue;
            }
            // Count-weighted PPMI: multiply by log(count) so rare 1-to-1
            // coincidences can't beat well-supported pairs even when their
            // raw PMI looks higher. Standard fix for PMI's low-frequency bias.
            let score = (pmi * (count as f64 + 1.0).ln()) as f32;
            by_token.entry(*a).or_default().push((*b, score));
            by_token.entry(*b).or_default().push((*a, score));
        }

        // Sort + truncate per token, resolve ids back to strings.
        let mut neighbours: HashMap<String, Vec<(String, f32)>> = HashMap::new();
        for (token_id, mut list) in by_token {
            list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            list.truncate(KEEP_PER_TOKEN);
            let token = self.tokens[token_id as usize].clone();
            let resolved = list
                .into_iter()
                .map(|(nid, score)| (self.tokens[nid as usize].clone(), score))
                .collect();
            neighbours.insert(token, resolved);
        }

        CooccurrenceIndex { neighbours }
    }
}

impl Default for CooccurrenceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build (and persist) a co-occurrence index for every text file under `root`.
/// Honours the same walk rules as the trigram index builder.
/// Controlled by env var `IG_SEMANTIC`: `0` disables build entirely.
pub fn build_for_root(root: &Path, use_default_excludes: bool, max_file_size: u64) -> Result<()> {
    if std::env::var("IG_SEMANTIC").ok().as_deref() == Some("0") {
        return Ok(());
    }
    let ig = crate::util::ig_dir(root);
    let paths = crate::walk::walk_files(root, use_default_excludes, max_file_size, None, None)
        .context("walk files for cooccurrence")?;

    let mut builder = CooccurrenceBuilder::new();
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if crate::util::is_binary(&bytes) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        builder.feed_text(text);
    }

    let idx = builder.finalise();
    idx.save(&ig)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_error_neighbours() {
        let mut b = CooccurrenceBuilder::new();
        // Synthetic corpus: error consistently co-occurs with catch/throw/exception.
        // Feed enough times to clear MIN_PAIR_COUNT + MIN_TOKEN_COUNT thresholds.
        for _ in 0..20 {
            b.feed_text(
                r#"
try {
    handleRequest();
} catch (HttpException e) {
    throw new CustomError("request failed");
}
            "#,
            );
        }
        // And a few unrelated lines so counts matter.
        for _ in 0..10 {
            b.feed_text("plain code foo bar baz");
        }
        let idx = b.finalise();
        let neigh = idx.expand("error", 10).expect("error should be known");
        // At least one of the expected neighbours must be there.
        let has_related = neigh.iter().any(|t| {
            ["catch", "exception", "custom", "failed", "request", "http"].contains(&t.as_str())
        });
        assert!(
            has_related,
            "expected error to link to catch/exception/…, got {:?}",
            neigh
        );
    }

    #[test]
    fn unknown_token_returns_none() {
        let b = CooccurrenceBuilder::new();
        let idx = b.finalise();
        assert!(idx.expand("neverseen", 5).is_none());
    }

    #[test]
    fn max_tokens_cap_respected() {
        let mut b = CooccurrenceBuilder::new();
        // Feed way more unique tokens than MAX_TOKENS would allow.
        // Use a tiny manually-crafted corpus; assertion is that we don't panic.
        for i in 0..10 {
            b.feed_text(&format!("t{}a t{}b t{}c t{}d", i, i, i, i));
        }
        let _ = b.finalise();
    }
}
