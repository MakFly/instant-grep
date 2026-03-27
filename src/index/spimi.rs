use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use ahash::AHashMap;
use anyhow::{Context, Result};

use super::ngram::NgramKey;
use super::postings::DocId;
use super::vbyte;

/// Magic bytes for segment files: "IGSG"
const SEGMENT_MAGIC: u32 = 0x4947_5347;
const SEGMENT_VERSION: u32 = 1;
const SEGMENT_HEADER_SIZE: usize = 16;

/// Default memory budget: 128 MB
pub const DEFAULT_MEMORY_BUDGET: usize = 128 * 1024 * 1024;

/// Tracks estimated heap memory used by the postings accumulator.
pub struct MemoryBudget {
    limit: usize,
    current: usize,
}

impl MemoryBudget {
    pub fn new(limit: usize) -> Self {
        Self { limit, current: 0 }
    }

    /// Track a new posting entry. `key_is_new` indicates whether this is a new
    /// ngram key (new HashMap entry) vs appending to an existing posting list.
    #[inline]
    pub fn track_posting(&mut self, key_is_new: bool) {
        if key_is_new {
            // AHashMap entry overhead (~80 bytes) + initial Vec allocation (~64 bytes)
            self.current += 144;
        }
        // Each DocId in the Vec: 4 bytes
        self.current += 4;
    }

    #[inline]
    pub fn should_flush(&self) -> bool {
        self.current >= self.limit
    }

    pub fn reset(&mut self) {
        self.current = 0;
    }

    #[cfg(test)]
    pub fn current(&self) -> usize {
        self.current
    }
}

/// Info about a written segment file.
pub struct SegmentInfo {
    pub path: PathBuf,
    #[allow(dead_code)]
    pub ngram_count: u32,
}

/// Flush the current postings map to a segment file.
pub fn flush_segment(
    postings_map: &mut AHashMap<NgramKey, Vec<DocId>>,
    segment_dir: &Path,
    segment_id: u32,
) -> Result<SegmentInfo> {
    // Sort entries by key for sequential merge later
    let mut entries: Vec<(NgramKey, Vec<DocId>)> = postings_map.drain().collect();
    entries.sort_unstable_by_key(|(k, _)| *k);

    let path = segment_dir.join(format!("seg_{:04}.bin", segment_id));
    let mut writer = BufWriter::new(File::create(&path).context("create segment file")?);

    // Write header
    writer.write_all(&SEGMENT_MAGIC.to_le_bytes())?;
    writer.write_all(&SEGMENT_VERSION.to_le_bytes())?;
    writer.write_all(&(entries.len() as u32).to_le_bytes())?;
    writer.write_all(&0u32.to_le_bytes())?; // reserved

    // Write sorted entries
    for (key, doc_ids) in &mut entries {
        doc_ids.sort_unstable();
        doc_ids.dedup();

        let encoded = vbyte::encode_posting_list(doc_ids);

        writer.write_all(&key.to_le_bytes())?;
        writer.write_all(&(encoded.len() as u32).to_le_bytes())?;
        writer.write_all(&encoded)?;
    }

    writer.flush()?;

    let ngram_count = entries.len() as u32;
    Ok(SegmentInfo {
        path,
        ngram_count,
    })
}

/// Read a segment file and iterate its entries.
pub struct SegmentReader {
    data: Vec<u8>,
    pos: usize,
    remaining: u32,
}

/// A single entry from a segment: ngram key + VByte-encoded posting list bytes.
pub struct SegmentEntry {
    pub key: NgramKey,
    pub posting_bytes: Vec<u8>,
}

impl SegmentReader {
    pub fn open(path: &Path) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("read segment {}", path.display()))?;

        // Validate header
        if data.len() < SEGMENT_HEADER_SIZE {
            anyhow::bail!("segment file too small: {}", path.display());
        }
        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != SEGMENT_MAGIC {
            anyhow::bail!("invalid segment magic in {}", path.display());
        }
        let ngram_count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        Ok(Self {
            data,
            pos: SEGMENT_HEADER_SIZE,
            remaining: ngram_count,
        })
    }

    /// Read the next entry. Returns None when exhausted.
    pub fn next_entry(&mut self) -> Option<SegmentEntry> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        let key = u64::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ]);
        self.pos += 8;

        let byte_len = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]) as usize;
        self.pos += 4;

        let posting_bytes = self.data[self.pos..self.pos + byte_len].to_vec();
        self.pos += byte_len;

        Some(SegmentEntry {
            key,
            posting_bytes,
        })
    }
}

#[cfg(test)]
/// Build SPIMI segments from file_data. Test helper — production code calls flush_segment directly.
pub fn build_segments(
    file_data: &[(String, u64, u64, Vec<NgramKey>)],
    memory_budget: usize,
    segment_dir: &Path,
) -> Result<Vec<SegmentInfo>> {
    fs::create_dir_all(segment_dir).context("create segment directory")?;

    let mut budget = MemoryBudget::new(memory_budget);
    let mut postings_map: AHashMap<NgramKey, Vec<DocId>> = AHashMap::new();
    let mut segments: Vec<SegmentInfo> = Vec::new();
    let mut segment_id: u32 = 0;

    for (new_id, (_rel_path, _size, _mtime, ngrams)) in file_data.iter().enumerate() {
        for &key in ngrams {
            let is_new = !postings_map.contains_key(&key);
            postings_map.entry(key).or_default().push(new_id as DocId);
            budget.track_posting(is_new);
        }

        if budget.should_flush() && !postings_map.is_empty() {
            let info = flush_segment(&mut postings_map, segment_dir, segment_id)?;
            segments.push(info);
            segment_id += 1;
            budget.reset();
        }
    }

    // Flush remaining postings
    if !postings_map.is_empty() {
        let info = flush_segment(&mut postings_map, segment_dir, segment_id)?;
        segments.push(info);
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_memory_budget() {
        let mut budget = MemoryBudget::new(1000);
        assert!(!budget.should_flush());

        // New key: 144 + 4 = 148 bytes
        budget.track_posting(true);
        assert_eq!(budget.current(), 148);

        // Existing key: +4 bytes
        budget.track_posting(false);
        assert_eq!(budget.current(), 152);

        budget.reset();
        assert_eq!(budget.current(), 0);
    }

    #[test]
    fn test_build_and_read_segment() {
        let tmp = TempDir::new().unwrap();
        let seg_dir = tmp.path().join("segments");

        let file_data: Vec<(String, u64, u64, Vec<NgramKey>)> = vec![
            ("a.rs".into(), 100, 0, vec![1, 2, 3]),
            ("b.rs".into(), 200, 0, vec![2, 3, 4]),
            ("c.rs".into(), 300, 0, vec![1, 4, 5]),
        ];

        let segments = build_segments(&file_data, DEFAULT_MEMORY_BUDGET, &seg_dir).unwrap();

        // With default budget, should produce 1 segment
        assert_eq!(segments.len(), 1);

        let mut reader = SegmentReader::open(&segments[0].path).unwrap();
        let mut entries = Vec::new();
        while let Some(entry) = reader.next_entry() {
            entries.push(entry);
        }

        // Should have 5 unique ngram keys (1,2,3,4,5)
        assert_eq!(entries.len(), 5);

        // Entries should be sorted by key
        for i in 1..entries.len() {
            assert!(entries[i - 1].key < entries[i].key);
        }

        // Verify posting lists
        let decode = |e: &SegmentEntry| vbyte::decode_posting_list(&e.posting_bytes, 0, e.posting_bytes.len());

        // Key 1: files a.rs (0) and c.rs (2)
        assert_eq!(decode(&entries[0]), vec![0, 2]);
        // Key 2: files a.rs (0) and b.rs (1)
        assert_eq!(decode(&entries[1]), vec![0, 1]);
        // Key 5: file c.rs (2) only
        assert_eq!(decode(&entries[4]), vec![2]);
    }

    #[test]
    fn test_multiple_segments() {
        let tmp = TempDir::new().unwrap();
        let seg_dir = tmp.path().join("segments");

        let file_data: Vec<(String, u64, u64, Vec<NgramKey>)> = vec![
            ("a.rs".into(), 100, 0, vec![1, 2, 3]),
            ("b.rs".into(), 200, 0, vec![2, 3, 4]),
        ];

        // Very small budget to force multiple segments
        let segments = build_segments(&file_data, 1, &seg_dir).unwrap();

        // Should produce 2 segments (one flush after each file)
        assert_eq!(segments.len(), 2);
    }
}
