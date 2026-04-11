use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;

use crate::index::metadata::IndexMetadata;
use crate::index::ngram::NgramKey;
use crate::index::overlay::OverlayReader;
use crate::index::postings::{self, DocId};
use crate::index::vbyte;
use crate::query::plan::NgramQuery;

const LEXICON_ENTRY_SIZE: usize = 18; // u64 key + u32 offset + u32 byte_length + u8 bloom + u8 loc_mask

pub struct IndexReader {
    pub metadata: IndexMetadata,
    lexicon: Mmap,
    postings: Mmap,
    table_size: usize,
    overlay: Option<OverlayReader>,
}

impl IndexReader {
    pub fn open(ig_dir: &Path) -> Result<Self> {
        let metadata = IndexMetadata::load_from(ig_dir).context("load metadata")?;

        let lexicon_file = File::open(ig_dir.join("lexicon.bin")).context("open lexicon.bin")?;
        let lexicon = unsafe { Mmap::map(&lexicon_file).context("mmap lexicon.bin")? };

        let postings_file = File::open(ig_dir.join("postings.bin")).context("open postings.bin")?;
        let postings = unsafe { Mmap::map(&postings_file).context("mmap postings.bin")? };

        // Hint the OS about expected access patterns for mmap'd files
        #[cfg(unix)]
        {
            unsafe {
                // Lexicon is small and always needed — prefault into page cache
                libc::madvise(
                    lexicon.as_ptr() as *mut libc::c_void,
                    lexicon.len(),
                    libc::MADV_WILLNEED,
                );
                // Postings are accessed randomly via offset lookups — disable readahead
                libc::madvise(
                    postings.as_ptr() as *mut libc::c_void,
                    postings.len(),
                    libc::MADV_RANDOM,
                );
            }
        }

        let table_size = lexicon.len() / LEXICON_ENTRY_SIZE;

        // Try to load overlay if it exists
        let overlay = OverlayReader::open(ig_dir).unwrap_or(None);

        Ok(Self {
            metadata,
            lexicon,
            postings,
            table_size,
            overlay,
        })
    }

    /// Look up a single n-gram key, merging base + overlay results.
    pub fn lookup_ngram(&self, key: NgramKey) -> Vec<DocId> {
        let mut results = self.lookup_ngram_base(key);

        if let Some(ref overlay) = self.overlay {
            // Filter tombstoned DocIds from base results
            results.retain(|&id| !overlay.is_tombstoned(id));

            // Add overlay results (DocIds are in disjoint range, so concat is fine)
            let overlay_results = overlay.lookup_ngram(key);
            if !overlay_results.is_empty() {
                results.extend_from_slice(&overlay_results);
            }
        }

        results
    }

    /// Look up in the base index only.
    /// Look up in the base index only — decode the full posting list.
    fn lookup_ngram_base(&self, key: NgramKey) -> Vec<DocId> {
        match self.lookup_raw(key) {
            Some((offset, byte_len)) => {
                vbyte::decode_posting_list(&self.postings, offset, byte_len)
            }
            None => Vec::new(),
        }
    }

    /// Look up a base ngram key and return raw posting location (offset, byte_len).
    /// Returns None if key not found.
    fn lookup_raw(&self, key: NgramKey) -> Option<(usize, usize)> {
        if self.table_size == 0 {
            return None;
        }
        let stored_key = key + 1;
        let start_slot = (stored_key as usize) % self.table_size;
        let mut slot = start_slot;
        let mut first = true;

        loop {
            if !first && slot == start_slot {
                return None;
            }
            first = false;
            let base = slot * LEXICON_ENTRY_SIZE;
            if base + LEXICON_ENTRY_SIZE > self.lexicon.len() {
                return None;
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
                return None;
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
                    return None;
                }
                return Some((offset, byte_len));
            }
            slot = (slot + 1) % self.table_size;
        }
    }

    /// Look up bloom and loc masks for a base ngram key.
    /// Returns (bloom_mask, loc_mask), or (0, 0) if key not found.
    #[allow(dead_code)]
    pub fn lookup_masks(&self, key: NgramKey) -> (u8, u8) {
        if self.table_size == 0 {
            return (0, 0);
        }
        let stored_key = key + 1;
        let start_slot = (stored_key as usize) % self.table_size;
        let mut slot = start_slot;
        let mut first = true;

        loop {
            if !first && slot == start_slot {
                return (0, 0);
            }
            first = false;
            let base = slot * LEXICON_ENTRY_SIZE;
            if base + LEXICON_ENTRY_SIZE > self.lexicon.len() {
                return (0, 0);
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
                return (0, 0);
            }
            if entry_key == stored_key {
                return (self.lexicon[base + 16], self.lexicon[base + 17]);
            }
            slot = (slot + 1) % self.table_size;
        }
    }

    /// Create a streaming PostingIterator over a base ngram's posting list.
    fn posting_iter(&self, key: NgramKey) -> vbyte::PostingIterator<'_> {
        match self.lookup_raw(key) {
            Some((offset, byte_len)) => {
                vbyte::PostingIterator::new(&self.postings, offset, byte_len)
            }
            None => vbyte::PostingIterator::new(&[], 0, 0),
        }
    }

    /// Resolve a full NgramQuery into candidate doc IDs.
    pub fn resolve(&self, query: &NgramQuery) -> Vec<DocId> {
        match query {
            NgramQuery::Ngram(key) => self.lookup_ngram(*key),
            NgramQuery::And(children) => self.resolve_and(children),
            NgramQuery::Or(children) => {
                if children.is_empty() {
                    return Vec::new();
                }
                let mut result = Vec::new();
                for child in children {
                    let list = self.resolve(child);
                    result = postings::union(&result, &list);
                }
                result
            }
            NgramQuery::All => {
                let total = self.total_file_count();
                (0..total).collect()
            }
        }
    }

    /// Streaming And resolution — avoids decoding all posting lists into Vec.
    /// Fast path when all children are Ngram leaves (common case for long literals).
    fn resolve_and(&self, children: &[NgramQuery]) -> Vec<DocId> {
        if children.is_empty() {
            return Vec::new();
        }

        // Fast path: all children are Ngram leaves → full streaming intersection
        let all_leaves = children.iter().all(|c| matches!(c, NgramQuery::Ngram(_)));

        if all_leaves && self.overlay.is_none() {
            return self.resolve_and_streaming(children);
        }

        if all_leaves && self.overlay.is_some() {
            // Streaming on base, then apply overlay
            return self.resolve_and_streaming_with_overlay(children);
        }

        // Fallback: mixed query types, use the old approach
        let mut lists: Vec<Vec<DocId>> = children.iter().map(|child| self.resolve(child)).collect();
        lists.sort_unstable_by_key(|l| l.len());
        let mut result = lists.remove(0);
        for list in &lists {
            result = postings::intersect(&result, list);
            if result.is_empty() {
                break;
            }
        }
        result
    }

    /// Pure streaming intersection on base index (no overlay).
    fn resolve_and_streaming(&self, children: &[NgramQuery]) -> Vec<DocId> {
        let keys: Vec<NgramKey> = children
            .iter()
            .map(|c| match c {
                NgramQuery::Ngram(k) => *k,
                _ => unreachable!(),
            })
            .collect();

        // Single key — just decode it directly
        if keys.len() == 1 {
            return self.lookup_ngram_base(keys[0]);
        }

        // Create iterators metadata, sort by doc_count (smallest first)
        let mut iters: Vec<(u32, NgramKey)> = keys
            .iter()
            .map(|&k| {
                let count = match self.lookup_raw(k) {
                    Some((offset, byte_len)) => {
                        vbyte::PostingIterator::new(&self.postings, offset, byte_len).doc_count()
                    }
                    None => 0,
                };
                (count, k)
            })
            .collect();
        iters.sort_unstable_by_key(|&(count, _)| count);

        // Early exit if any list is empty
        if iters[0].0 == 0 {
            return Vec::new();
        }

        // Intersect pairwise: first two as iterators, then chain
        let mut it_a = self.posting_iter(iters[0].1);
        let mut it_b = self.posting_iter(iters[1].1);
        let mut result = postings::intersect_two_iters(&mut it_a, &mut it_b);

        // Chain remaining iterators against the materialized result
        for &(_, key) in &iters[2..] {
            if result.is_empty() {
                break;
            }
            let mut it = self.posting_iter(key);
            result = postings::intersect_vec_iter(&result, &mut it);
        }

        result
    }

    /// Streaming intersection on base, then apply overlay (tombstones + overlay docs).
    fn resolve_and_streaming_with_overlay(&self, children: &[NgramQuery]) -> Vec<DocId> {
        // Phase 1: streaming intersection on base posting lists
        let mut base_result = self.resolve_and_streaming(children);

        let overlay = self.overlay.as_ref().unwrap();

        // Phase 2: filter tombstoned docs
        base_result.retain(|&id| !overlay.is_tombstoned(id));

        // Phase 3: intersect overlay posting lists (small, ok to materialize)
        let keys: Vec<NgramKey> = children
            .iter()
            .map(|c| match c {
                NgramQuery::Ngram(k) => *k,
                _ => unreachable!(),
            })
            .collect();

        let mut overlay_lists: Vec<Vec<DocId>> =
            keys.iter().map(|&k| overlay.lookup_ngram(k)).collect();
        overlay_lists.sort_unstable_by_key(|l| l.len());

        if !overlay_lists.is_empty() && !overlay_lists[0].is_empty() {
            let mut overlay_result = overlay_lists.remove(0);
            for list in &overlay_lists {
                overlay_result = postings::intersect(&overlay_result, list);
                if overlay_result.is_empty() {
                    break;
                }
            }
            // Merge base + overlay (both sorted, disjoint ID ranges)
            debug_assert!(
                base_result.last().is_none_or(|&last| overlay_result
                    .first()
                    .is_none_or(|&first| last < first)),
                "overlay IDs must be greater than all base IDs"
            );
            base_result.extend_from_slice(&overlay_result);
        }

        base_result
    }

    /// Get file path for a DocId, checking overlay for high DocIds.
    pub fn file_path(&self, doc_id: DocId) -> &str {
        if let Some(ref overlay) = self.overlay
            && doc_id >= self.metadata.file_count
        {
            let overlay_idx = (doc_id - self.metadata.file_count) as usize;
            if overlay_idx < overlay.metadata.added_files.len() {
                return &overlay.metadata.added_files[overlay_idx].path;
            }
        }
        &self.metadata.files[doc_id as usize].path
    }

    /// Pre-fault the postings mmap into the OS page cache via sequential scan.
    pub fn warm_postings(&self) {
        #[cfg(unix)]
        unsafe {
            // Temporarily set sequential for readahead
            libc::madvise(
                self.postings.as_ptr() as *mut libc::c_void,
                self.postings.len(),
                libc::MADV_SEQUENTIAL,
            );
        }
        // Touch one byte per page to trigger page faults
        let page_size = 4096;
        let mut sum: u8 = 0;
        for offset in (0..self.postings.len()).step_by(page_size) {
            sum = sum.wrapping_add(self.postings[offset]);
        }
        std::hint::black_box(sum);
        #[cfg(unix)]
        unsafe {
            // Reset to random for query-time access
            libc::madvise(
                self.postings.as_ptr() as *mut libc::c_void,
                self.postings.len(),
                libc::MADV_RANDOM,
            );
        }
    }

    /// Total file count including overlay.
    pub fn total_file_count(&self) -> u32 {
        if let Some(ref overlay) = self.overlay {
            overlay.total_file_count()
        } else {
            self.metadata.file_count
        }
    }
}
