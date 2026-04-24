//! BM25 ranking for file-level search results.
//!
//! Uses the Okapi BM25 formula treating the user's pattern as a single term
//! and each matched file as a document:
//!   score = IDF · (tf · (k1 + 1)) / (tf + k1 · (1 − b + b · dl / avdl))
//!
//! `tf`       → `match_count` (number of pattern hits in the file)
//! `df`       → number of files in the current result set with ≥1 match
//! `dl`       → file length in bytes
//! `avdl`     → mean file length across the result set
//! `N`        → |result set| (we don't need the full corpus N; ranking only
//!              compares files that already matched)
//!
//! This gives us a cheap, deterministic "most informative first" ordering
//! without any ML model. Combined with `--top N`, the agent receives the
//! K best-ranked files instead of the first K found on disk.

use std::path::Path;

use crate::search::matcher::FileMatches;

const K1: f32 = 1.5;
const B: f32 = 0.75;

/// Rank results in-place by BM25 score and truncate to `top`.
/// `root` is the search root used to resolve relative file paths for `stat()`.
pub fn rank_top(results: &mut Vec<FileMatches>, root: &Path, top: usize) {
    if results.is_empty() {
        return;
    }

    let lens: Vec<u64> = results
        .iter()
        .map(|fm| {
            std::fs::metadata(root.join(&fm.path))
                .map(|m| m.len().max(1))
                .unwrap_or(1)
        })
        .collect();

    let n = results.len() as f32;
    let df = n; // every file in `results` matched at least once
    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.01);
    let avdl = lens.iter().sum::<u64>() as f32 / n;

    // Pair each FileMatches with its score, sort, take top N.
    let mut scored: Vec<(f32, FileMatches)> = results
        .drain(..)
        .zip(lens.iter())
        .map(|(fm, &dl)| {
            let tf = fm.match_count as f32;
            let dl_f = dl as f32;
            let denom = tf + K1 * (1.0 - B + B * dl_f / avdl);
            let score = idf * (tf * (K1 + 1.0)) / denom.max(f32::EPSILON);
            (score, fm)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top);

    *results = scored.into_iter().map(|(_, fm)| fm).collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_file(dir: &Path, name: &str, size: usize) -> String {
        let path = dir.join(name);
        let content = "x".repeat(size);
        fs::write(&path, content).unwrap();
        name.to_string()
    }

    #[test]
    fn dense_small_file_beats_sparse_large_file() {
        let dir = tempdir().unwrap();
        // small file, many matches  →  high tf, short dl  →  high score
        let small = make_file(dir.path(), "small.rs", 500);
        // large file, few matches   →  low tf, long dl    →  low score
        let large = make_file(dir.path(), "large.rs", 50_000);

        let mut results = vec![
            FileMatches {
                path: large,
                matches: Vec::new(),
                match_count: 1,
            },
            FileMatches {
                path: small,
                matches: Vec::new(),
                match_count: 20,
            },
        ];

        rank_top(&mut results, dir.path(), 10);
        assert_eq!(results[0].path, "small.rs");
        assert_eq!(results[1].path, "large.rs");
    }

    #[test]
    fn truncates_to_top_n() {
        let dir = tempdir().unwrap();
        let mut results: Vec<FileMatches> = (0..5)
            .map(|i| FileMatches {
                path: make_file(dir.path(), &format!("f{}.rs", i), 100 * (i + 1)),
                matches: Vec::new(),
                match_count: i + 1,
            })
            .collect();

        rank_top(&mut results, dir.path(), 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn under_top_keeps_all_but_sorts() {
        let dir = tempdir().unwrap();
        let mut results = vec![
            FileMatches {
                path: make_file(dir.path(), "a.rs", 200),
                matches: Vec::new(),
                match_count: 1,
            },
            FileMatches {
                path: make_file(dir.path(), "b.rs", 200),
                matches: Vec::new(),
                match_count: 10,
            },
        ];
        rank_top(&mut results, dir.path(), 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].path, "b.rs");
    }
}
