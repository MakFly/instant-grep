//! Brute-force cosine similarity search over the JSON store.
//! OpenAI vectors are L2-normalised → cosine = dot product.

use crate::embed_poc::store::{Store, StoredChunk};
use rayon::prelude::*;

#[derive(Debug)]
pub struct SearchHit<'a> {
    pub score: f32,
    pub chunk: &'a StoredChunk,
}

pub fn search<'a>(store: &'a Store, query_vec: &[f32], top_n: usize) -> Vec<SearchHit<'a>> {
    if store.chunks.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<SearchHit> = store
        .chunks
        .par_iter()
        .map(|c| SearchHit {
            score: dot(&c.embedding, query_vec),
            chunk: c,
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_n);
    scored
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut acc = 0.0f32;
    for i in 0..n {
        acc += a[i] * b[i];
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed_poc::store::{Store, StoredChunk};

    fn mk_store() -> Store {
        use std::f32::consts::FRAC_1_SQRT_2 as H;
        let mut s = Store::new("openai", "test", 3);
        for (i, e) in [
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
            vec![H, H, 0.0],
        ]
        .into_iter()
        .enumerate()
        {
            s.chunks.push(StoredChunk {
                id: i as u32,
                file: format!("f{}.rs", i),
                start_line: 1,
                end_line: 1,
                tokens: 1,
                embedding: e,
            });
        }
        s
    }

    #[test]
    fn exact_match_top1() {
        let store = mk_store();
        let q = vec![1.0, 0.0, 0.0];
        let hits = search(&store, &q, 5);
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0].chunk.id, 0);
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn diagonal_query_picks_45deg_chunk() {
        let store = mk_store();
        let q = vec![
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ];
        let hits = search(&store, &q, 1);
        assert_eq!(hits[0].chunk.id, 3);
    }

    #[test]
    fn empty_store_returns_empty() {
        let store = Store::new("openai", "test", 3);
        let hits = search(&store, &[1.0, 0.0, 0.0], 5);
        assert!(hits.is_empty());
    }
}
