//! Naive 40-line chunker with 5-line overlap. Phase 2 — pedagogical, not production.
//! Phase 4+ would switch to symbol-aware chunking via `src/symbols.rs`.

use std::path::{Path, PathBuf};

pub const CHUNK_LINES: usize = 40;
pub const OVERLAP_LINES: usize = 5;

#[derive(Debug, Clone)]
pub struct RawChunk {
    pub file: PathBuf,
    pub start_line: usize, // 1-indexed, inclusive
    pub end_line: usize,   // 1-indexed, inclusive
    pub text: String,
}

/// Split a file's content into chunks. 1-indexed line numbers, inclusive end.
pub fn chunk_file(path: &Path, content: &str) -> Vec<RawChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let stride = CHUNK_LINES.saturating_sub(OVERLAP_LINES).max(1);

    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let text = lines[start..end].join("\n");
        chunks.push(RawChunk {
            file: path.to_path_buf(),
            start_line: start + 1,
            end_line: end,
            text,
        });
        if end == lines.len() {
            break;
        }
        start += stride;
    }

    chunks
}

/// Estimate tokens by 4-char heuristic (OpenAI tiktoken averages ~3.5–4 chars/token on code).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() / 4).max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_yields_no_chunks() {
        let chunks = chunk_file(Path::new("a.rs"), "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn short_file_one_chunk() {
        let content = (1..=10)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_file(Path::new("a.rs"), &content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 10);
    }

    #[test]
    fn long_file_overlapping_chunks() {
        // 100 lines → with stride 35: chunks at [1-40], [36-75], [71-100]
        let content = (1..=100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_file(Path::new("a.rs"), &content);
        assert_eq!(chunks.len(), 3);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 40));
        assert_eq!((chunks[1].start_line, chunks[1].end_line), (36, 75));
        assert_eq!((chunks[2].start_line, chunks[2].end_line), (71, 100));
    }

    #[test]
    fn token_estimate_smoke() {
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("12345678"), 2);
    }
}
