use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

use super::ngram::NgramKey;
use super::spimi::{SegmentInfo, SegmentReader};
use super::vbyte;

/// Result of merging segments: per-ngram offset info for building the lexicon.
pub struct MergedEntry {
    pub key: NgramKey,
    pub byte_offset: u32,
    pub byte_length: u32,
}

/// Entry in the min-heap for k-way merge.
#[derive(Eq, PartialEq)]
struct HeapEntry {
    key: NgramKey,
    segment_idx: usize,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap (BinaryHeap is max-heap)
        other.key.cmp(&self.key).then(other.segment_idx.cmp(&self.segment_idx))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Merge all segment files into final postings.bin.
///
/// Returns the merged entries (key, byte_offset, byte_length) for lexicon construction.
pub fn merge_segments(
    segments: &[SegmentInfo],
    postings_path: &Path,
) -> Result<Vec<MergedEntry>> {
    if segments.is_empty() {
        // Write empty postings file
        File::create(postings_path).context("create empty postings.bin")?;
        return Ok(Vec::new());
    }

    // Single segment: fast path — just read and write directly
    if segments.len() == 1 {
        return merge_single_segment(&segments[0], postings_path);
    }

    // Open all segment readers
    let mut readers: Vec<SegmentReader> = segments
        .iter()
        .map(|s| SegmentReader::open(&s.path))
        .collect::<Result<_>>()?;

    // Peek buffer: current entry from each reader
    let mut current_entries: Vec<Option<super::spimi::SegmentEntry>> =
        Vec::with_capacity(readers.len());

    // Initialize: read first entry from each segment, populate heap
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    for (idx, reader) in readers.iter_mut().enumerate() {
        let entry = reader.next_entry();
        if let Some(ref e) = entry {
            heap.push(HeapEntry {
                key: e.key,
                segment_idx: idx,
            });
        }
        current_entries.push(entry);
    }

    let mut postings_writer =
        BufWriter::new(File::create(postings_path).context("create postings.bin")?);
    let mut merged: Vec<MergedEntry> = Vec::new();
    let mut current_offset: u32 = 0;

    while let Some(min_entry) = heap.pop() {
        let min_key = min_entry.key;

        // Collect all posting bytes for this key across segments
        // Since DocId ranges are disjoint across segments (sequential processing),
        // we can simply concatenate the decoded lists.
        let mut all_doc_ids: Vec<u32> = Vec::new();

        // Process the segment that was popped
        {
            let idx = min_entry.segment_idx;
            if let Some(ref entry) = current_entries[idx] {
                let ids = vbyte::decode_posting_list(&entry.posting_bytes, 0, entry.posting_bytes.len());
                all_doc_ids.extend_from_slice(&ids);
            }
            // Advance this reader
            let next = readers[idx].next_entry();
            if let Some(ref e) = next {
                heap.push(HeapEntry {
                    key: e.key,
                    segment_idx: idx,
                });
            }
            current_entries[idx] = next;
        }

        // Check if other segments also have this same key
        while let Some(peek) = heap.peek() {
            if peek.key != min_key {
                break;
            }
            let entry = heap.pop().unwrap();
            let idx = entry.segment_idx;
            if let Some(ref seg_entry) = current_entries[idx] {
                let ids = vbyte::decode_posting_list(&seg_entry.posting_bytes, 0, seg_entry.posting_bytes.len());
                all_doc_ids.extend_from_slice(&ids);
            }
            // Advance this reader
            let next = readers[idx].next_entry();
            if let Some(ref e) = next {
                heap.push(HeapEntry {
                    key: e.key,
                    segment_idx: idx,
                });
            }
            current_entries[idx] = next;
        }

        // DocIds should already be sorted (disjoint ranges from sequential processing)
        // but sort to be safe
        all_doc_ids.sort_unstable();
        all_doc_ids.dedup();

        // Encode and write
        let encoded = vbyte::encode_posting_list(&all_doc_ids);
        let byte_len = encoded.len() as u32;
        postings_writer.write_all(&encoded)?;

        merged.push(MergedEntry {
            key: min_key,
            byte_offset: current_offset,
            byte_length: byte_len,
        });
        current_offset += byte_len;
    }

    postings_writer.flush()?;
    Ok(merged)
}

/// Fast path for a single segment: just re-encode as postings.bin.
fn merge_single_segment(segment: &SegmentInfo, postings_path: &Path) -> Result<Vec<MergedEntry>> {
    let mut reader = SegmentReader::open(&segment.path)?;
    let mut postings_writer =
        BufWriter::new(File::create(postings_path).context("create postings.bin")?);
    let mut merged: Vec<MergedEntry> = Vec::new();
    let mut current_offset: u32 = 0;

    while let Some(entry) = reader.next_entry() {
        let byte_len = entry.posting_bytes.len() as u32;
        postings_writer.write_all(&entry.posting_bytes)?;

        merged.push(MergedEntry {
            key: entry.key,
            byte_offset: current_offset,
            byte_length: byte_len,
        });
        current_offset += byte_len;
    }

    postings_writer.flush()?;
    Ok(merged)
}

/// Build the lexicon hash table from merged entries.
///
/// Returns the table as a byte vector ready to write to lexicon.bin.
pub fn build_lexicon(entries: &[MergedEntry]) -> Vec<u8> {
    let table_size = next_prime((entries.len() as f64 * 1.3) as usize);
    if table_size == 0 {
        return Vec::new();
    }

    const ENTRY_SIZE: usize = 16; // u64 key + u32 offset + u32 length
    let mut table = vec![0u8; table_size * ENTRY_SIZE];

    for entry in entries {
        let stored_key = entry.key + 1; // sentinel: 0 = empty
        let mut slot = (stored_key as usize) % table_size;
        loop {
            let base = slot * ENTRY_SIZE;
            let existing = u64::from_le_bytes([
                table[base],
                table[base + 1],
                table[base + 2],
                table[base + 3],
                table[base + 4],
                table[base + 5],
                table[base + 6],
                table[base + 7],
            ]);
            if existing == 0 {
                table[base..base + 8].copy_from_slice(&stored_key.to_le_bytes());
                table[base + 8..base + 12].copy_from_slice(&entry.byte_offset.to_le_bytes());
                table[base + 12..base + 16].copy_from_slice(&entry.byte_length.to_le_bytes());
                break;
            }
            slot = (slot + 1) % table_size;
        }
    }

    table
}

/// Clean up temporary segment files.
pub fn cleanup_segments(segments: &[SegmentInfo]) {
    for seg in segments {
        let _ = fs::remove_file(&seg.path);
    }
    // Try to remove the segment directory (only succeeds if empty)
    if let Some(seg) = segments.first() {
        if let Some(parent) = seg.path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

fn next_prime(n: usize) -> usize {
    if n <= 2 {
        return 2;
    }
    let mut candidate = if n % 2 == 0 { n + 1 } else { n };
    loop {
        if is_prime(candidate) {
            return candidate;
        }
        candidate += 2;
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n == 2 || n == 3 {
        return true;
    }
    if n % 2 == 0 || n % 3 == 0 {
        return false;
    }
    let mut i = 5;
    while i * i <= n {
        if n % i == 0 || n % (i + 2) == 0 {
            return false;
        }
        i += 6;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::spimi;
    use tempfile::TempDir;

    #[test]
    fn test_merge_single_segment() {
        let tmp = TempDir::new().unwrap();
        let seg_dir = tmp.path().join("segments");
        let postings_path = tmp.path().join("postings.bin");

        let file_data: Vec<(String, u64, u64, Vec<NgramKey>)> = vec![
            ("a.rs".into(), 100, 0, vec![10, 20, 30]),
            ("b.rs".into(), 200, 0, vec![20, 30, 40]),
        ];

        let segments = spimi::build_segments(&file_data, spimi::DEFAULT_MEMORY_BUDGET, &seg_dir).unwrap();
        let merged = merge_segments(&segments, &postings_path).unwrap();

        // Should have 4 unique keys: 10, 20, 30, 40
        assert_eq!(merged.len(), 4);

        // Verify sorted order
        for i in 1..merged.len() {
            assert!(merged[i - 1].key < merged[i].key);
        }

        // Verify we can decode the postings
        let postings_data = fs::read(&postings_path).unwrap();
        for entry in &merged {
            let ids = vbyte::decode_posting_list(
                &postings_data,
                entry.byte_offset as usize,
                entry.byte_length as usize,
            );
            assert!(!ids.is_empty());
        }

        // Key 20: should have doc_ids [0, 1]
        let key20 = merged.iter().find(|e| e.key == 20).unwrap();
        let ids = vbyte::decode_posting_list(
            &postings_data,
            key20.byte_offset as usize,
            key20.byte_length as usize,
        );
        assert_eq!(ids, vec![0, 1]);
    }

    #[test]
    fn test_merge_multiple_segments() {
        let tmp = TempDir::new().unwrap();
        let seg_dir = tmp.path().join("segments");
        let postings_path = tmp.path().join("postings.bin");

        let file_data: Vec<(String, u64, u64, Vec<NgramKey>)> = vec![
            ("a.rs".into(), 100, 0, vec![10, 20]),
            ("b.rs".into(), 200, 0, vec![20, 30]),
        ];

        // Force 2 segments with tiny budget
        let segments = spimi::build_segments(&file_data, 1, &seg_dir).unwrap();
        assert!(segments.len() >= 2);

        let merged = merge_segments(&segments, &postings_path).unwrap();

        // Should have 3 unique keys: 10, 20, 30
        assert_eq!(merged.len(), 3);

        let postings_data = fs::read(&postings_path).unwrap();

        // Key 20: should have doc_ids [0, 1] (from both segments)
        let key20 = merged.iter().find(|e| e.key == 20).unwrap();
        let ids = vbyte::decode_posting_list(
            &postings_data,
            key20.byte_offset as usize,
            key20.byte_length as usize,
        );
        assert_eq!(ids, vec![0, 1]);
    }

    #[test]
    fn test_build_lexicon() {
        let entries = vec![
            MergedEntry { key: 10, byte_offset: 0, byte_length: 5 },
            MergedEntry { key: 20, byte_offset: 5, byte_length: 8 },
            MergedEntry { key: 30, byte_offset: 13, byte_length: 3 },
        ];

        let table = build_lexicon(&entries);
        assert!(!table.is_empty());

        // Verify we can look up each key
        let table_size = table.len() / 16;
        for entry in &entries {
            let stored_key = entry.key + 1;
            let mut slot = (stored_key as usize) % table_size;
            loop {
                let base = slot * 16;
                let found_key = u64::from_le_bytes([
                    table[base], table[base+1], table[base+2], table[base+3],
                    table[base+4], table[base+5], table[base+6], table[base+7],
                ]);
                if found_key == stored_key {
                    let offset = u32::from_le_bytes([
                        table[base+8], table[base+9], table[base+10], table[base+11],
                    ]);
                    let length = u32::from_le_bytes([
                        table[base+12], table[base+13], table[base+14], table[base+15],
                    ]);
                    assert_eq!(offset, entry.byte_offset);
                    assert_eq!(length, entry.byte_length);
                    break;
                }
                slot = (slot + 1) % table_size;
            }
        }
    }

    #[test]
    fn test_merge_empty() {
        let tmp = TempDir::new().unwrap();
        let postings_path = tmp.path().join("postings.bin");
        let merged = merge_segments(&[], &postings_path).unwrap();
        assert!(merged.is_empty());
    }
}
