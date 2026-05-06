use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use memmap2::Mmap;

use crate::index::metadata::IndexMetadata;
use crate::index::ngram::{
    NgramKey, POSITION_ZONE_COUNT, POSITION_ZONE_OVERFLOW_BIT, POSITION_ZONE_SIZE, loc_bit,
};
use crate::index::overlay::OverlayReader;
use crate::index::postings::{self, DocId};
use crate::index::vbyte::{self, PostingEntry};
use crate::query::plan::NgramQuery;

const LEXICON_ENTRY_SIZE: usize = 18; // u64 key + u32 offset + u32 byte_length + u8 bloom + u8 loc_mask
struct MaskedList {
    #[allow(dead_code)]
    key: NgramKey,
    rel_pos: u16,
    exact_pos: bool,
    entries: Vec<PostingEntry>,
}

#[derive(Clone, Copy)]
struct LeafSpec {
    key: NgramKey,
    next_mask: u8,
    rel_pos: u16,
    exact_pos: bool,
    estimated_cost: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveStats {
    pub ngram_leaves: usize,
    pub raw_postings: usize,
    pub decoded_postings: usize,
    pub after_bloom: usize,
    pub after_intersection: usize,
    pub after_loc: usize,
    pub bloom_rejects: usize,
    pub loc_rejects: usize,
    pub skip_blocks_used: usize,
    pub skip_blocks_visited: usize,
    pub block_mask_rejects: usize,
    pub zone_rejects: usize,
}

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

        validate_metadata(&metadata)?;

        let lexicon_path = ig_dir.join("lexicon.bin");
        let postings_path = ig_dir.join("postings.bin");
        let lexicon_meta = std::fs::metadata(&lexicon_path).context("stat lexicon.bin")?;
        let postings_meta = std::fs::metadata(&postings_path).context("stat postings.bin")?;
        validate_artifact_sizes(&metadata, lexicon_meta.len(), postings_meta.len())?;

        let lexicon_file = File::open(&lexicon_path).context("open lexicon.bin")?;
        let lexicon = unsafe { Mmap::map(&lexicon_file).context("mmap lexicon.bin")? };

        let postings_file = File::open(&postings_path).context("open postings.bin")?;
        let postings = unsafe { Mmap::map(&postings_file).context("mmap postings.bin")? };

        // Hint the OS about expected access patterns for mmap'd files.
        // Avoid MADV_WILLNEED: a stale or huge cache entry can force the whole
        // lexicon through disk before a query can fail or fall back.
        #[cfg(unix)]
        {
            unsafe {
                // Postings are accessed randomly via offset lookups — disable readahead
                libc::madvise(
                    postings.as_ptr() as *mut libc::c_void,
                    postings.len(),
                    libc::MADV_RANDOM,
                );
            }
        }

        let table_size = lexicon.len() / LEXICON_ENTRY_SIZE;

        // Try to load overlay if it exists. Log open errors instead of
        // collapsing them to None: silent swallows masked a daemon bug where a
        // mid-write overlay would stay invisible until process restart.
        let overlay = match OverlayReader::open(ig_dir) {
            Ok(opt) => opt,
            Err(e) => {
                eprintln!(
                    "[ig] overlay open failed in {}: {:#} (serving base only)",
                    ig_dir.display(),
                    e
                );
                None
            }
        };

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
        self.lookup_entries(key)
            .into_iter()
            .map(|entry| entry.doc_id)
            .collect()
    }

    fn lookup_entries(&self, key: NgramKey) -> Vec<PostingEntry> {
        let mut results = self.lookup_entries_base(key);

        if let Some(ref overlay) = self.overlay {
            // Filter tombstoned DocIds from base results
            results.retain(|entry| !overlay.is_tombstoned(entry.doc_id));

            // Add overlay results (DocIds are in disjoint range, so concat is fine)
            let overlay_results = overlay.lookup_entries(key);
            if !overlay_results.is_empty() {
                results.extend_from_slice(&overlay_results);
            }
        }

        results
    }

    /// Look up in the base index only — decode the full posting list.
    fn lookup_entries_base(&self, key: NgramKey) -> Vec<PostingEntry> {
        match self.lookup_raw(key) {
            Some((offset, byte_len)) => {
                vbyte::decode_posting_entries(&self.postings, offset, byte_len)
            }
            None => Vec::new(),
        }
    }

    fn lookup_masked_entries_with_counts(
        &self,
        key: NgramKey,
        next_mask: u8,
    ) -> (Vec<PostingEntry>, usize, usize) {
        let logical_count = self.lookup_entry_count(key);
        let mut entries = self.lookup_entries(key);
        let decoded_count = entries.len();
        if next_mask != 0 {
            entries.retain(|entry| entry.next_mask & next_mask != 0);
        }
        (entries, logical_count, decoded_count)
    }

    fn lookup_entry_count(&self, key: NgramKey) -> usize {
        let mut count = self
            .lookup_raw(key)
            .map(|(offset, byte_len)| vbyte::posting_entry_count(&self.postings, offset, byte_len))
            .unwrap_or(0);
        if let Some(ref overlay) = self.overlay {
            count += overlay.lookup_entries(key).len();
        }
        count
    }

    fn lookup_masked_entries_for_docs(
        &self,
        spec: LeafSpec,
        candidate_docs: &[DocId],
    ) -> (Vec<PostingEntry>, usize, usize, usize, usize, usize) {
        if candidate_docs.is_empty() {
            return (Vec::new(), 0, 0, 0, 0, 0);
        }

        let mut decoded_count = 0usize;
        let mut after_bloom = 0usize;
        let mut visited_blocks = 0usize;
        let mut block_mask_rejects = 0usize;
        let mut matches = Vec::new();

        if let Some((offset, byte_len)) = self.lookup_raw(spec.key) {
            if vbyte::is_skip_encoded(&self.postings, offset, byte_len) {
                let mut skipper = vbyte::PostingEntrySkipper::new(&self.postings, offset, byte_len);
                for &candidate in candidate_docs {
                    let (entry, rejected_block) =
                        skipper.advance_to_masked(candidate, spec.next_mask, 0, 0);
                    if rejected_block {
                        block_mask_rejects += 1;
                    }
                    if let Some(entry) = entry {
                        visited_blocks = visited_blocks.max(skipper.visited_blocks());
                        decoded_count = decoded_count.max(skipper.decoded_entries());
                        if entry.doc_id == candidate
                            && !self.is_tombstoned(entry.doc_id)
                            && (spec.next_mask == 0 || entry.next_mask & spec.next_mask != 0)
                        {
                            after_bloom += 1;
                            matches.push(entry);
                        }
                    }
                }
            } else {
                let mut entries = self.lookup_entries_base(spec.key);
                decoded_count += entries.len();
                if spec.next_mask != 0 {
                    entries.retain(|entry| entry.next_mask & spec.next_mask != 0);
                }
                after_bloom += entries.len();
                for entry in entries {
                    if candidate_docs.binary_search(&entry.doc_id).is_ok()
                        && !self.is_tombstoned(entry.doc_id)
                    {
                        matches.push(entry);
                    }
                }
            }
        }

        if let Some(ref overlay) = self.overlay {
            for entry in overlay.lookup_entries(spec.key) {
                decoded_count += 1;
                if spec.next_mask != 0 && entry.next_mask & spec.next_mask == 0 {
                    continue;
                }
                after_bloom += 1;
                if candidate_docs.binary_search(&entry.doc_id).is_ok() {
                    matches.push(entry);
                }
            }
        }

        (
            matches,
            self.lookup_entry_count(spec.key),
            decoded_count,
            after_bloom,
            visited_blocks,
            block_mask_rejects,
        )
    }

    fn is_tombstoned(&self, doc_id: DocId) -> bool {
        self.overlay
            .as_ref()
            .map(|overlay| overlay.is_tombstoned(doc_id))
            .unwrap_or(false)
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

    pub fn estimate_query_cost(&self, query: &NgramQuery) -> u64 {
        match query {
            NgramQuery::Ngram(key) => self.estimate_ngram_cost(*key),
            NgramQuery::MaskedNgram { key, next_mask, .. } => {
                let base = self.estimate_ngram_cost(*key);
                if *next_mask == 0 {
                    base
                } else {
                    // The 8-bit next-byte bloom is lossy. In expectation, one
                    // hashed follow byte passes around 1/8 of uniformly mixed
                    // postings; use a conservative 1/4 so planning does not
                    // over-trust the filter on code-like distributions.
                    base / 4 + 1
                }
            }
            NgramQuery::And(children) => {
                if children.is_empty() {
                    return 0;
                }
                let mut sum = 0u64;
                let mut min = u64::MAX;
                for child in children {
                    let cost = self.estimate_query_cost(child);
                    sum = sum.saturating_add(cost);
                    min = min.min(cost);
                }
                // Decode cost is roughly additive. Candidate cost is bounded
                // by the rarest posting list, so add it as a small proxy for
                // downstream verification work.
                sum.saturating_add(min)
            }
            NgramQuery::Or(children) => children
                .iter()
                .map(|child| self.estimate_query_cost(child))
                .fold(0u64, u64::saturating_add),
            NgramQuery::All => u64::MAX / 4,
        }
    }

    fn estimate_ngram_cost(&self, key: NgramKey) -> u64 {
        let mut cost = self
            .lookup_raw(key)
            .map(|(_, byte_len)| byte_len as u64)
            .unwrap_or(0);
        if let Some(ref overlay) = self.overlay {
            let overlay_entries = overlay.lookup_entries(key).len() as u64;
            // Overlay entries are already decoded from a compact side index.
            // Keep the unit close to bytes so base and overlay costs combine.
            cost = cost.saturating_add(overlay_entries.saturating_mul(6));
        }
        cost
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

    /// Resolve a full NgramQuery into candidate doc IDs.
    pub fn resolve(&self, query: &NgramQuery) -> Vec<DocId> {
        self.resolve_with_stats(query).0
    }

    pub fn resolve_with_stats(&self, query: &NgramQuery) -> (Vec<DocId>, ResolveStats) {
        match query {
            NgramQuery::Ngram(key) => {
                let entries = self.lookup_entries(*key);
                let len = entries.len();
                (
                    entries.into_iter().map(|entry| entry.doc_id).collect(),
                    ResolveStats {
                        ngram_leaves: 1,
                        raw_postings: self.lookup_entry_count(*key),
                        decoded_postings: len,
                        after_bloom: len,
                        after_intersection: len,
                        after_loc: len,
                        ..ResolveStats::default()
                    },
                )
            }
            NgramQuery::MaskedNgram { key, next_mask, .. } => {
                let (entries, raw_count, decoded_count) =
                    self.lookup_masked_entries_with_counts(*key, *next_mask);
                let len = entries.len();
                (
                    entries.into_iter().map(|entry| entry.doc_id).collect(),
                    ResolveStats {
                        ngram_leaves: 1,
                        raw_postings: raw_count,
                        decoded_postings: decoded_count,
                        after_bloom: len,
                        after_intersection: len,
                        after_loc: len,
                        bloom_rejects: decoded_count.saturating_sub(len),
                        ..ResolveStats::default()
                    },
                )
            }
            NgramQuery::And(children) => self.resolve_and_with_stats(children),
            NgramQuery::Or(children) => {
                if children.is_empty() {
                    return (Vec::new(), ResolveStats::default());
                }
                let mut result = Vec::new();
                let mut stats = ResolveStats::default();
                for child in children {
                    let (list, child_stats) = self.resolve_with_stats(child);
                    stats.ngram_leaves += child_stats.ngram_leaves;
                    stats.raw_postings += child_stats.raw_postings;
                    stats.decoded_postings += child_stats.decoded_postings;
                    stats.after_bloom += child_stats.after_bloom;
                    stats.bloom_rejects += child_stats.bloom_rejects;
                    stats.loc_rejects += child_stats.loc_rejects;
                    stats.skip_blocks_used += child_stats.skip_blocks_used;
                    stats.skip_blocks_visited += child_stats.skip_blocks_visited;
                    stats.block_mask_rejects += child_stats.block_mask_rejects;
                    stats.zone_rejects += child_stats.zone_rejects;
                    result = postings::union(&result, &list);
                }
                stats.after_intersection = result.len();
                stats.after_loc = result.len();
                (result, stats)
            }
            NgramQuery::All => {
                let total = self.total_file_count();
                let all: Vec<_> = (0..total).collect();
                (
                    all,
                    ResolveStats {
                        after_intersection: total as usize,
                        after_loc: total as usize,
                        ..ResolveStats::default()
                    },
                )
            }
        }
    }

    fn resolve_and_with_stats(&self, children: &[NgramQuery]) -> (Vec<DocId>, ResolveStats) {
        if children.is_empty() {
            return (Vec::new(), ResolveStats::default());
        }

        let all_leaves = children
            .iter()
            .all(|c| matches!(c, NgramQuery::Ngram(_) | NgramQuery::MaskedNgram { .. }));

        if all_leaves {
            return self.resolve_and_masked_with_stats(children);
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
        let len = result.len();
        (
            result,
            ResolveStats {
                after_intersection: len,
                after_loc: len,
                ..ResolveStats::default()
            },
        )
    }

    fn resolve_and_masked_with_stats(&self, children: &[NgramQuery]) -> (Vec<DocId>, ResolveStats) {
        let mut stats = ResolveStats {
            ngram_leaves: children.len(),
            ..ResolveStats::default()
        };

        let mut specs: Vec<LeafSpec> = children
            .iter()
            .map(|child| match child {
                NgramQuery::Ngram(key) => LeafSpec {
                    key: *key,
                    next_mask: 0,
                    rel_pos: 0,
                    exact_pos: false,
                    estimated_cost: self.estimate_ngram_cost(*key),
                },
                NgramQuery::MaskedNgram {
                    key,
                    next_mask,
                    rel_pos,
                    exact_pos,
                } => LeafSpec {
                    key: *key,
                    next_mask: *next_mask,
                    rel_pos: *rel_pos,
                    exact_pos: *exact_pos,
                    estimated_cost: self.estimate_query_cost(child),
                },
                _ => unreachable!(),
            })
            .collect();
        specs.sort_unstable_by_key(|spec| spec.estimated_cost);
        if specs.is_empty() {
            return (Vec::new(), stats);
        }

        let first = specs[0];
        let (first_entries, first_raw_count, first_decoded_count) =
            self.lookup_masked_entries_with_counts(first.key, first.next_mask);
        stats.raw_postings += first_raw_count;
        stats.decoded_postings += first_decoded_count;
        stats.after_bloom += first_entries.len();
        stats.bloom_rejects += first_decoded_count.saturating_sub(first_entries.len());
        if first_entries.is_empty() {
            return (Vec::new(), stats);
        }

        let mut lists = vec![MaskedList {
            key: first.key,
            rel_pos: first.rel_pos,
            exact_pos: first.exact_pos,
            entries: first_entries,
        }];
        let mut result: Vec<DocId> = lists[0].entries.iter().map(|entry| entry.doc_id).collect();

        for spec in &specs[1..] {
            let use_skip = self
                .lookup_raw(spec.key)
                .map(|(offset, byte_len)| {
                    let entry_count = vbyte::posting_entry_count(&self.postings, offset, byte_len);
                    vbyte::is_skip_encoded(&self.postings, offset, byte_len)
                        && result.len().saturating_mul(4) < entry_count
                })
                .unwrap_or(false);

            let entries = if use_skip {
                let (
                    entries,
                    raw_count,
                    decoded_count,
                    after_bloom,
                    visited_blocks,
                    block_mask_rejects,
                ) = self.lookup_masked_entries_for_docs(*spec, &result);
                stats.raw_postings += raw_count;
                stats.decoded_postings += decoded_count;
                stats.after_bloom += after_bloom;
                stats.bloom_rejects += decoded_count.saturating_sub(after_bloom);
                stats.skip_blocks_used += 1;
                stats.skip_blocks_visited += visited_blocks;
                stats.block_mask_rejects += block_mask_rejects;
                entries
            } else {
                let (entries, raw_count, decoded_count) =
                    self.lookup_masked_entries_with_counts(spec.key, spec.next_mask);
                stats.raw_postings += raw_count;
                stats.decoded_postings += decoded_count;
                stats.after_bloom += entries.len();
                stats.bloom_rejects += decoded_count.saturating_sub(entries.len());
                entries
            };
            let ids: Vec<DocId> = entries.iter().map(|entry| entry.doc_id).collect();
            result = postings::intersect(&result, &ids);
            lists.push(MaskedList {
                key: spec.key,
                rel_pos: spec.rel_pos,
                exact_pos: spec.exact_pos,
                entries,
            });
            if result.is_empty() {
                break;
            }
        }
        stats.after_intersection = result.len();

        if lists.len() > 1 {
            let before_loc = result.len();
            result.retain(|&doc_id| loc_masks_are_compatible(doc_id, &lists));
            stats.loc_rejects = before_loc.saturating_sub(result.len());
            if exact_position_filter_is_safe(&lists) {
                let before_zone = result.len();
                result.retain(|&doc_id| zone_masks_are_compatible(doc_id, &lists));
                stats.zone_rejects = before_zone.saturating_sub(result.len());
            }
        }
        stats.after_loc = result.len();
        (result, stats)
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

    /// Total file count including overlay.
    pub fn total_file_count(&self) -> u32 {
        if let Some(ref overlay) = self.overlay {
            overlay.total_file_count()
        } else {
            self.metadata.file_count
        }
    }
}

fn validate_metadata(metadata: &IndexMetadata) -> Result<()> {
    if metadata.files.len() != metadata.file_count as usize {
        bail!(
            "invalid index metadata: file_count={} but files has {} entries",
            metadata.file_count,
            metadata.files.len()
        );
    }
    Ok(())
}

fn loc_masks_are_compatible(doc_id: DocId, lists: &[MaskedList]) -> bool {
    for base_mod in 0..8usize {
        let mut ok = true;
        for list in lists {
            let Some(entry) = posting_for_doc(&list.entries, doc_id) else {
                ok = false;
                break;
            };
            if entry.loc_mask == 0 {
                continue;
            }
            let required = loc_bit((base_mod + list.rel_pos as usize) % 8);
            if entry.loc_mask & required == 0 {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}

fn exact_position_filter_is_safe(lists: &[MaskedList]) -> bool {
    lists.len() >= 2 && lists.iter().all(|list| list.exact_pos)
}

fn zone_masks_are_compatible(doc_id: DocId, lists: &[MaskedList]) -> bool {
    if lists.iter().all(|list| list.exact_pos) {
        return exact_position_masks_are_compatible(doc_id, lists);
    }

    for base_zone in 0..POSITION_ZONE_COUNT {
        let mut ok = true;
        for list in lists {
            let Some(entry) = posting_for_doc(&list.entries, doc_id) else {
                ok = false;
                break;
            };
            if entry.zone_mask == 0 || entry.zone_mask & POSITION_ZONE_OVERFLOW_BIT != 0 {
                continue;
            }
            let compatible = compatible_zone_mask(base_zone, list.rel_pos as usize);
            if entry.zone_mask & compatible == 0 {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}

fn exact_position_masks_are_compatible(doc_id: DocId, lists: &[MaskedList]) -> bool {
    for list in lists {
        let Some(entry) = posting_for_doc(&list.entries, doc_id) else {
            return false;
        };
        if entry.zone_mask == 0 || entry.zone_mask & POSITION_ZONE_OVERFLOW_BIT != 0 {
            continue;
        }
        let required = compatible_zone_mask(0, list.rel_pos as usize);
        if entry.zone_mask & required == 0 {
            return false;
        }
    }
    true
}

fn compatible_zone_mask(base_zone: usize, rel_pos: usize) -> u32 {
    let start = base_zone
        .saturating_mul(POSITION_ZONE_SIZE)
        .saturating_add(rel_pos);
    let end = base_zone
        .saturating_mul(POSITION_ZONE_SIZE)
        .saturating_add(POSITION_ZONE_SIZE - 1)
        .saturating_add(rel_pos);
    let first_zone = start / POSITION_ZONE_SIZE;
    let last_zone = end / POSITION_ZONE_SIZE;
    let mut mask = 0u32;
    for zone in first_zone..=last_zone {
        if zone < POSITION_ZONE_COUNT {
            mask |= 1u32 << zone;
        } else {
            mask |= POSITION_ZONE_OVERFLOW_BIT;
        }
    }
    mask
}

fn posting_for_doc(entries: &[PostingEntry], doc_id: DocId) -> Option<PostingEntry> {
    entries
        .binary_search_by_key(&doc_id, |entry| entry.doc_id)
        .ok()
        .map(|idx| entries[idx])
}

fn validate_artifact_sizes(
    metadata: &IndexMetadata,
    lexicon_bytes: u64,
    postings_bytes: u64,
) -> Result<()> {
    if !lexicon_bytes.is_multiple_of(LEXICON_ENTRY_SIZE as u64) {
        bail!(
            "invalid lexicon.bin size: {} bytes is not a multiple of {}",
            lexicon_bytes,
            LEXICON_ENTRY_SIZE
        );
    }

    let table_slots = lexicon_bytes / LEXICON_ENTRY_SIZE as u64;
    let ngrams = metadata.ngram_count as u64;
    if ngrams == 0 {
        if lexicon_bytes != 0 || postings_bytes != 0 {
            bail!(
                "invalid empty index artifacts: ngram_count=0 but lexicon={} bytes, postings={} bytes",
                lexicon_bytes,
                postings_bytes
            );
        }
        return Ok(());
    }

    if table_slots < ngrams {
        bail!(
            "invalid lexicon.bin: {} slots for {} ngrams",
            table_slots,
            ngrams
        );
    }

    let max_expected_slots = ngrams.saturating_mul(2).saturating_add(1024);
    if table_slots > max_expected_slots {
        bail!(
            "invalid lexicon.bin: {} slots for {} ngrams",
            table_slots,
            ngrams
        );
    }

    if postings_bytes == 0 {
        bail!("invalid postings.bin: empty postings for {} ngrams", ngrams);
    }

    Ok(())
}
