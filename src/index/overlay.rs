use std::fs::{self, File};
use std::io::{BufWriter, Write as IoWrite};
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};

use super::merge;
use super::metadata::IndexedFile;
use super::ngram::NgramKey;
use super::postings::DocId;
use super::vbyte::{self, PostingEntry};

/// Metadata for an overlay index (small incremental layer on top of base).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayMetadata {
    pub version: u32,
    pub base_file_count: u32,
    pub base_git_commit: Option<String>,
    pub overlay_git_commit: Option<String>,
    pub added_files: Vec<IndexedFile>,
    pub tombstone_doc_ids: Vec<DocId>,
    pub overlay_file_count: u32,
    pub overlay_ngram_count: u32,
}

/// Reader for overlay index files.
pub struct OverlayReader {
    pub metadata: OverlayMetadata,
    lexicon: Mmap,
    postings: Mmap,
    table_size: usize,
    tombstone_bits: Vec<u8>,
}

const LEXICON_ENTRY_SIZE: usize = 18;

impl OverlayReader {
    /// Open an overlay index from .ig/ directory. Returns None if no overlay exists.
    pub fn open(ig_dir: &Path) -> Result<Option<Self>> {
        let meta_path = ig_dir.join("overlay_meta.bin");
        if !meta_path.exists() {
            return Ok(None);
        }

        let data = fs::read(&meta_path).context("read overlay_meta.bin")?;
        let metadata: OverlayMetadata =
            bincode::deserialize(&data).context("deserialize overlay_meta.bin")?;

        let lex_path = ig_dir.join("overlay_lex.bin");
        let post_path = ig_dir.join("overlay.bin");
        let tomb_path = ig_dir.join("tombstones.bin");

        if !lex_path.exists() || !post_path.exists() {
            return Ok(None);
        }

        let lex_file = File::open(&lex_path).context("open overlay_lex.bin")?;
        let lexicon = unsafe { Mmap::map(&lex_file).context("mmap overlay_lex.bin")? };

        let post_file = File::open(&post_path).context("open overlay.bin")?;
        let postings = unsafe { Mmap::map(&post_file).context("mmap overlay.bin")? };

        let table_size = lexicon.len() / LEXICON_ENTRY_SIZE;

        let tombstone_bits = if tomb_path.exists() {
            fs::read(&tomb_path).context("read tombstones.bin")?
        } else {
            Vec::new()
        };

        Ok(Some(Self {
            metadata,
            lexicon,
            postings,
            table_size,
            tombstone_bits,
        }))
    }

    /// Check if a base DocId is tombstoned (invalidated by overlay).
    #[inline]
    pub fn is_tombstoned(&self, doc_id: DocId) -> bool {
        let byte_idx = (doc_id / 8) as usize;
        let bit_idx = doc_id % 8;
        if byte_idx < self.tombstone_bits.len() {
            (self.tombstone_bits[byte_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Look up a single n-gram key in the overlay hash table.
    #[allow(dead_code)]
    pub fn lookup_ngram(&self, key: NgramKey) -> Vec<DocId> {
        self.lookup_entries(key)
            .into_iter()
            .map(|entry| entry.doc_id)
            .collect()
    }

    pub fn lookup_entries(&self, key: NgramKey) -> Vec<PostingEntry> {
        if self.table_size == 0 {
            return Vec::new();
        }

        let stored_key = key + 1;
        let mut slot = (stored_key as usize) % self.table_size;

        loop {
            let base = slot * LEXICON_ENTRY_SIZE;
            if base + LEXICON_ENTRY_SIZE > self.lexicon.len() {
                return Vec::new();
            }

            let entry_key = u64::from_le_bytes([
                self.lexicon[base],
                self.lexicon[base + 1],
                self.lexicon[base + 2],
                self.lexicon[base + 3],
                self.lexicon[base + 4],
                self.lexicon[base + 5],
                self.lexicon[base + 6],
                self.lexicon[base + 7],
            ]);

            if entry_key == 0 {
                return Vec::new();
            }

            if entry_key == stored_key {
                let offset = u32::from_le_bytes([
                    self.lexicon[base + 8],
                    self.lexicon[base + 9],
                    self.lexicon[base + 10],
                    self.lexicon[base + 11],
                ]) as usize;
                let byte_len = u32::from_le_bytes([
                    self.lexicon[base + 12],
                    self.lexicon[base + 13],
                    self.lexicon[base + 14],
                    self.lexicon[base + 15],
                ]) as usize;

                let end = offset + byte_len;
                if end > self.postings.len() {
                    return Vec::new();
                }

                return vbyte::decode_posting_entries(&self.postings, offset, byte_len);
            }

            slot = (slot + 1) % self.table_size;
        }
    }

    /// Total file count including overlay additions.
    pub fn total_file_count(&self) -> u32 {
        self.metadata.base_file_count + self.metadata.overlay_file_count
    }

    /// Whether the overlay is large enough to warrant compaction (full rebuild).
    pub fn needs_compaction(&self, base_file_count: u32) -> bool {
        self.metadata.overlay_file_count > 1000
            || (base_file_count > 0 && self.metadata.overlay_file_count * 10 > base_file_count)
    }
}

/// Build an overlay index for a set of changed files.
///
/// `base_file_count`: number of files in the base index (for DocId offset).
/// `base_files`: the base index file list (to find tombstone DocIds for modified/deleted files).
/// `changed_file_data`: (rel_path, size, mtime, ngrams with masks) for new/modified files.
/// `deleted_paths`: relative paths of files deleted since base index.
pub fn build_overlay(
    ig_dir: &Path,
    base_file_count: u32,
    base_files: &[IndexedFile],
    changed_file_data: &[(String, u64, u64, Vec<(NgramKey, u8, u8, u32)>)],
    deleted_paths: &[String],
    base_git_commit: &Option<String>,
    overlay_git_commit: &Option<String>,
) -> Result<OverlayMetadata> {
    // Find tombstone DocIds: base files that were modified or deleted
    let changed_paths: std::collections::HashSet<&str> = changed_file_data
        .iter()
        .map(|(p, _, _, _)| p.as_str())
        .chain(deleted_paths.iter().map(|s| s.as_str()))
        .collect();

    let mut tombstone_doc_ids: Vec<DocId> = Vec::new();
    for (doc_id, file) in base_files.iter().enumerate() {
        if changed_paths.contains(file.path.as_str()) {
            tombstone_doc_ids.push(doc_id as DocId);
        }
    }

    // Build tombstone bitmap
    let tombstone_bytes = if !tombstone_doc_ids.is_empty() {
        let max_id = *tombstone_doc_ids.iter().max().unwrap();
        let byte_count = (max_id / 8 + 1) as usize;
        let mut bits = vec![0u8; byte_count];
        for &id in &tombstone_doc_ids {
            let byte_idx = (id / 8) as usize;
            let bit_idx = id % 8;
            bits[byte_idx] |= 1 << bit_idx;
        }
        bits
    } else {
        Vec::new()
    };

    // Build postings for overlay files with DocIds starting at base_file_count
    let mut postings_map: ahash::AHashMap<NgramKey, Vec<PostingEntry>> = ahash::AHashMap::new();
    let mut added_files: Vec<IndexedFile> = Vec::new();

    for (idx, (rel_path, size, mtime, ngrams)) in changed_file_data.iter().enumerate() {
        let doc_id = base_file_count + idx as u32;
        added_files.push(IndexedFile {
            path: rel_path.clone(),
            mtime: *mtime,
            size: *size,
        });
        for &(key, bloom, loc, zone) in ngrams {
            postings_map.entry(key).or_default().push(PostingEntry {
                doc_id,
                next_mask: bloom,
                loc_mask: loc,
                zone_mask: zone,
            });
        }
    }

    // Sort and dedup posting lists
    for list in postings_map.values_mut() {
        list.sort_unstable_by_key(|p| p.doc_id);
        list.dedup_by_key(|p| p.doc_id);
    }

    // Sort by key and build postings + lexicon
    let mut sorted_entries: Vec<(NgramKey, &Vec<PostingEntry>)> = postings_map
        .iter()
        .map(|(&key, list)| (key, list))
        .collect();
    sorted_entries.sort_unstable_by_key(|(key, _)| *key);

    let postings_path = ig_dir.join("overlay.bin.tmp");
    let mut postings_writer =
        BufWriter::new(File::create(&postings_path).context("create overlay.bin")?);
    let mut merged_entries: Vec<merge::MergedEntry> = Vec::new();
    let mut current_offset: u32 = 0;

    for (key, postings) in &sorted_entries {
        let encoded = vbyte::encode_posting_entries(postings);
        let byte_len = encoded.len() as u32;
        let bloom_mask = postings.iter().fold(0u8, |acc, p| acc | p.next_mask);
        let loc_mask = postings.iter().fold(0u8, |acc, p| acc | p.loc_mask);
        postings_writer.write_all(&encoded)?;
        merged_entries.push(merge::MergedEntry {
            key: *key,
            byte_offset: current_offset,
            byte_length: byte_len,
            bloom_mask,
            loc_mask,
        });
        current_offset += byte_len;
    }
    postings_writer.flush()?;

    // Build and write overlay lexicon
    let lexicon_data = merge::build_lexicon(&merged_entries);
    fs::write(ig_dir.join("overlay_lex.bin.tmp"), &lexicon_data)
        .context("write overlay_lex.bin")?;

    // Write tombstones
    fs::write(ig_dir.join("tombstones.bin.tmp"), &tombstone_bytes)
        .context("write tombstones.bin")?;

    // Write overlay metadata
    let overlay_meta = OverlayMetadata {
        version: super::metadata::INDEX_VERSION,
        base_file_count,
        base_git_commit: base_git_commit.clone(),
        overlay_git_commit: overlay_git_commit.clone(),
        added_files,
        tombstone_doc_ids,
        overlay_file_count: changed_file_data.len() as u32,
        overlay_ngram_count: sorted_entries.len() as u32,
    };

    let encoded = bincode::serialize(&overlay_meta).context("serialize overlay_meta")?;
    fs::write(ig_dir.join("overlay_meta.bin.tmp"), &encoded).context("write overlay_meta.bin")?;

    // Publish atomically. `overlay_meta.bin` is renamed last because readers use
    // its mtime as the reload signal.
    fs::rename(ig_dir.join("overlay.bin.tmp"), ig_dir.join("overlay.bin"))
        .context("publish overlay.bin")?;
    fs::rename(
        ig_dir.join("overlay_lex.bin.tmp"),
        ig_dir.join("overlay_lex.bin"),
    )
    .context("publish overlay_lex.bin")?;
    fs::rename(
        ig_dir.join("tombstones.bin.tmp"),
        ig_dir.join("tombstones.bin"),
    )
    .context("publish tombstones.bin")?;
    fs::rename(
        ig_dir.join("overlay_meta.bin.tmp"),
        ig_dir.join("overlay_meta.bin"),
    )
    .context("publish overlay_meta.bin")?;

    Ok(overlay_meta)
}

/// Remove overlay files from .ig/ directory.
pub fn clear_overlay(ig_dir: &Path) {
    let _ = fs::remove_file(ig_dir.join("overlay.bin"));
    let _ = fs::remove_file(ig_dir.join("overlay_lex.bin"));
    let _ = fs::remove_file(ig_dir.join("overlay_meta.bin"));
    let _ = fs::remove_file(ig_dir.join("tombstones.bin"));
    let _ = fs::remove_file(ig_dir.join("overlay.bin.tmp"));
    let _ = fs::remove_file(ig_dir.join("overlay_lex.bin.tmp"));
    let _ = fs::remove_file(ig_dir.join("overlay_meta.bin.tmp"));
    let _ = fs::remove_file(ig_dir.join("tombstones.bin.tmp"));
}
