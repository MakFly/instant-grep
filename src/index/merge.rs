use std::collections::BinaryHeap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use memmap2::{Mmap, MmapMut};

use super::ngram::NgramKey;
use super::spimi::{SegmentInfo, SegmentReader};
use super::vbyte;

/// Result of merging segments: per-ngram offset info for building the lexicon.
pub struct MergedEntry {
    pub key: NgramKey,
    pub byte_offset: u32,
    pub byte_length: u32,
}

/// Size of a single entry in the temp file: u64 key + u32 offset + u32 length.
const ENTRY_FILE_SIZE: usize = 16;

/// Result of streaming merge: entries written to a temp file instead of a Vec.
pub struct StreamingMergeResult {
    pub entries_path: PathBuf,
    pub entry_count: usize,
}

impl Drop for StreamingMergeResult {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.entries_path);
    }
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
        other
            .key
            .cmp(&self.key)
            .then(other.segment_idx.cmp(&self.segment_idx))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Merge all segment files into final postings.bin (streaming / low-RAM).
///
/// Writes each MergedEntry to a temp file as it's produced instead of collecting
/// in a Vec. Returns the temp file path + entry count. The temp file is
/// automatically deleted when the `StreamingMergeResult` is dropped.
///
/// Format: 16 bytes per entry — key(u64 LE) + byte_offset(u32 LE) + byte_length(u32 LE).
pub fn merge_segments_streaming(
    segments: &[SegmentInfo],
    postings_path: &Path,
) -> Result<StreamingMergeResult> {
    // Create temp file for entries in same directory as postings (same filesystem)
    let entries_dir = postings_path.parent().unwrap_or(Path::new("."));
    let entries_file = tempfile::Builder::new()
        .prefix(".ig-entries-")
        .suffix(".tmp")
        .tempfile_in(entries_dir)
        .context("create entries temp file")?;
    let entries_path = entries_file.path().to_path_buf();
    // Keep the file by persisting (we manage cleanup via StreamingMergeResult::Drop)
    let entries_file = entries_file
        .persist(&entries_path)
        .context("persist entries temp file")?;

    if segments.is_empty() {
        File::create(postings_path).context("create empty postings.bin")?;
        return Ok(StreamingMergeResult {
            entries_path,
            entry_count: 0,
        });
    }

    if segments.len() == 1 {
        return merge_single_segment_streaming(
            &segments[0],
            postings_path,
            entries_file,
            entries_path,
        );
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
    let mut entries_writer = BufWriter::new(entries_file);
    let mut entry_count: usize = 0;
    let mut current_offset: u32 = 0;

    while let Some(min_entry) = heap.pop() {
        let min_key = min_entry.key;

        // Collect all posting bytes for this key across segments
        let mut all_doc_ids: Vec<u32> = Vec::new();

        // Process the segment that was popped
        {
            let idx = min_entry.segment_idx;
            if let Some(ref entry) = current_entries[idx] {
                let ids =
                    vbyte::decode_posting_list(&entry.posting_bytes, 0, entry.posting_bytes.len());
                all_doc_ids.extend_from_slice(&ids);
            }
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
                let ids = vbyte::decode_posting_list(
                    &seg_entry.posting_bytes,
                    0,
                    seg_entry.posting_bytes.len(),
                );
                all_doc_ids.extend_from_slice(&ids);
            }
            let next = readers[idx].next_entry();
            if let Some(ref e) = next {
                heap.push(HeapEntry {
                    key: e.key,
                    segment_idx: idx,
                });
            }
            current_entries[idx] = next;
        }

        all_doc_ids.sort_unstable();
        all_doc_ids.dedup();

        // Encode and write postings
        let encoded = vbyte::encode_posting_list(&all_doc_ids);
        let byte_len = encoded.len() as u32;
        postings_writer.write_all(&encoded)?;

        // Write entry to temp file: key(u64 LE) + offset(u32 LE) + length(u32 LE)
        write_entry(&mut entries_writer, min_key, current_offset, byte_len)?;
        entry_count += 1;
        current_offset += byte_len;
    }

    postings_writer.flush()?;
    entries_writer.flush()?;

    Ok(StreamingMergeResult {
        entries_path,
        entry_count,
    })
}

/// Fast path for a single segment: streaming variant.
fn merge_single_segment_streaming(
    segment: &SegmentInfo,
    postings_path: &Path,
    entries_file: File,
    entries_path: PathBuf,
) -> Result<StreamingMergeResult> {
    let mut reader = SegmentReader::open(&segment.path)?;
    let mut postings_writer =
        BufWriter::new(File::create(postings_path).context("create postings.bin")?);
    let mut entries_writer = BufWriter::new(entries_file);
    let mut entry_count: usize = 0;
    let mut current_offset: u32 = 0;

    while let Some(entry) = reader.next_entry() {
        let byte_len = entry.posting_bytes.len() as u32;
        postings_writer.write_all(&entry.posting_bytes)?;

        write_entry(&mut entries_writer, entry.key, current_offset, byte_len)?;
        entry_count += 1;
        current_offset += byte_len;
    }

    postings_writer.flush()?;
    entries_writer.flush()?;

    Ok(StreamingMergeResult {
        entries_path,
        entry_count,
    })
}

/// Write a single entry to the entries temp file.
#[inline]
fn write_entry(w: &mut BufWriter<File>, key: NgramKey, offset: u32, length: u32) -> Result<()> {
    w.write_all(&key.to_le_bytes())?;
    w.write_all(&offset.to_le_bytes())?;
    w.write_all(&length.to_le_bytes())?;
    Ok(())
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

/// Build the lexicon hash table from entries stored in a temp file (streaming path).
///
/// Mmaps the entries file for reading and builds the lexicon via mmap — no heap
/// allocation for either the entries or the hash table.
pub fn build_lexicon_mmap_from_file(
    entries_path: &Path,
    entry_count: usize,
    lexicon_path: &Path,
) -> Result<()> {
    if entry_count == 0 {
        File::create(lexicon_path).context("create empty lexicon.bin")?;
        return Ok(());
    }

    let table_size = next_prime((entry_count as f64 * 1.3) as usize);
    const ENTRY_SIZE: usize = 16;
    let byte_size = table_size * ENTRY_SIZE;

    // Mmap the entries file for reading
    let entries_file = File::open(entries_path).context("open entries temp file for lexicon")?;
    let entries_mmap = unsafe { Mmap::map(&entries_file).context("mmap entries temp file")? };

    // Create lexicon file with exact size, mmap it as writable
    let lex_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(lexicon_path)
        .context("create lexicon.bin")?;
    lex_file
        .set_len(byte_size as u64)
        .context("set lexicon.bin size")?;

    let mut mmap = unsafe { MmapMut::map_mut(&lex_file).context("mmap lexicon.bin for write")? };

    for i in 0..entry_count {
        let base = i * ENTRY_FILE_SIZE;
        let key = u64::from_le_bytes(entries_mmap[base..base + 8].try_into().unwrap());
        let byte_offset_bytes: [u8; 4] = entries_mmap[base + 8..base + 12].try_into().unwrap();
        let byte_length_bytes: [u8; 4] = entries_mmap[base + 12..base + 16].try_into().unwrap();

        let stored_key = key + 1; // sentinel: 0 = empty
        let mut slot = (stored_key as usize) % table_size;
        loop {
            let lex_base = slot * ENTRY_SIZE;
            let existing = u64::from_le_bytes(mmap[lex_base..lex_base + 8].try_into().unwrap());
            if existing == 0 {
                mmap[lex_base..lex_base + 8].copy_from_slice(&stored_key.to_le_bytes());
                mmap[lex_base + 8..lex_base + 12].copy_from_slice(&byte_offset_bytes);
                mmap[lex_base + 12..lex_base + 16].copy_from_slice(&byte_length_bytes);
                break;
            }
            slot = (slot + 1) % table_size;
        }
    }

    mmap.flush().context("flush lexicon mmap")?;
    Ok(())
}

/// Clean up temporary segment files.
pub fn cleanup_segments(segments: &[SegmentInfo]) {
    for seg in segments {
        let _ = fs::remove_file(&seg.path);
    }
    // Try to remove the segment directory (only succeeds if empty)
    if let Some(seg) = segments.first()
        && let Some(parent) = seg.path.parent()
    {
        let _ = fs::remove_dir(parent);
    }
}

fn next_prime(n: usize) -> usize {
    if n <= 2 {
        return 2;
    }
    let mut candidate = if n.is_multiple_of(2) { n + 1 } else { n };
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
    if n.is_multiple_of(2) || n.is_multiple_of(3) {
        return false;
    }
    let mut i = 5;
    while i * i <= n {
        if n.is_multiple_of(i) || n.is_multiple_of(i + 2) {
            return false;
        }
        i += 6;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_lexicon() {
        let entries = vec![
            MergedEntry {
                key: 10,
                byte_offset: 0,
                byte_length: 5,
            },
            MergedEntry {
                key: 20,
                byte_offset: 5,
                byte_length: 8,
            },
            MergedEntry {
                key: 30,
                byte_offset: 13,
                byte_length: 3,
            },
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
                    table[base],
                    table[base + 1],
                    table[base + 2],
                    table[base + 3],
                    table[base + 4],
                    table[base + 5],
                    table[base + 6],
                    table[base + 7],
                ]);
                if found_key == stored_key {
                    let offset = u32::from_le_bytes([
                        table[base + 8],
                        table[base + 9],
                        table[base + 10],
                        table[base + 11],
                    ]);
                    let length = u32::from_le_bytes([
                        table[base + 12],
                        table[base + 13],
                        table[base + 14],
                        table[base + 15],
                    ]);
                    assert_eq!(offset, entry.byte_offset);
                    assert_eq!(length, entry.byte_length);
                    break;
                }
                slot = (slot + 1) % table_size;
            }
        }
    }
}
