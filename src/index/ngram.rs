use std::collections::VecDeque;
use std::path::Path;

use ahash::AHashMap;

use crate::index::metadata::IndexMetadata;

/// Hash of a variable-length n-gram, used as index key.
pub type NgramKey = u64;

/// Default max n-gram length for covering algorithm.
pub const DEFAULT_MAX_NGRAM_LEN: usize = 16;

/// Hash a bigram (2 consecutive bytes) using Murmur2-like constants.
/// Port of danlark1/sparse_ngrams HashBigram.
#[inline]
pub(crate) fn hash_bigram(a: u8, b: u8) -> u32 {
    const MUL1: u64 = 0xc6a4a7935bd1e995;
    const MUL2: u64 = 0x228876a7198b743;
    let v = (a as u64)
        .wrapping_mul(MUL1)
        .wrapping_add((b as u64).wrapping_mul(MUL2));
    (v.wrapping_add(!v >> 47)) as u32
}

/// Corpus-tuned bigram weight for the sparse n-gram algorithm.
///
/// Wraps `hash_bigram` with frequency-based adjustments so that n-gram
/// boundaries land on semantically meaningful positions (word boundaries,
/// structural delimiters, rare character pairs) rather than at arbitrary
/// hash-maximum positions.
///
/// Key insight: weight = inverse of expected frequency.
///   - Rare pairs   → high weight → n-gram boundary placed here → more selective keys
///   - Common pairs  → low weight  → absorbed into interior of an n-gram
///
/// CRITICAL: this function MUST be identical at index-time and query-time.
#[inline]
fn bigram_weight(a: u8, b: u8) -> u32 {
    let base = hash_bigram(a, b) as u64;

    // --- Boost rare / structurally meaningful pairs (2x) ---

    // camelCase boundary: lowercase followed by uppercase
    let camel = a.is_ascii_lowercase() && b.is_ascii_uppercase();

    // snake_case boundary: lowercase followed by underscore
    let snake = a.is_ascii_lowercase() && b == b'_';

    if camel || snake {
        return (base.wrapping_mul(2) & 0xFFFF_FFFF) as u32;
    }

    // --- Boost structural delimiters (3/2x = 1.5x) ---
    // Braces and parens carry high information content in code.
    let structural = matches!(b, b'{' | b'}' | b'(' | b')');
    if structural {
        return (base.wrapping_mul(3).wrapping_div(2) & 0xFFFF_FFFF) as u32;
    }

    // --- Penalise high-frequency pairs (0.5x) ---

    // space + lowercase letter (very common in prose and code)
    let space_lower = a == b' ' && b.is_ascii_lowercase();

    // newline + space/tab (indentation lines — extremely frequent)
    let indent = a == b'\n' && (b == b' ' || b == b'\t');

    // double-space
    let double_space = a == b' ' && b == b' ';

    // common English / code bigrams
    let common_pair = matches!(
        (a, b),
        (b'e', b' ')
            | (b't', b' ')
            | (b'i', b'n')
            | (b'e', b'r')
            | (b'r', b'e')
            | (b'o', b'n')
            | (b't', b'h')
            | (b'h', b'e')
            | (b'a', b'n')
    );

    if space_lower || indent || double_space || common_pair {
        return (base.wrapping_div(2)) as u32;
    }

    base as u32
}

/// Compute bloom mask bit for a follow character (Cursor "3.5-gram" technique).
/// At index time, for trigram "abc" followed by "d", set bit `bloom_bit(d)`.
/// At query time, for pattern "abcd", check if bloom_mask for "abc" has the bit set.
#[inline]
pub fn bloom_bit(follow_char: u8) -> u8 {
    1u8 << (follow_char.wrapping_mul(31) % 8)
}

/// Compute locMask bit for a byte position (Cursor locMask technique).
/// At index time, for trigram at byte position p, set bit `loc_bit(p)`.
/// At query time, check if adjacent trigrams' position masks could be adjacent.
#[inline]
pub fn loc_bit(position: usize) -> u8 {
    1u8 << (position % 8)
}

pub const POSITION_ZONE_SIZE: usize = 1;
pub const POSITION_ZONE_COUNT: usize = 31;
pub const POSITION_ZONE_OVERFLOW_BIT: u32 = 1u32 << 31;

/// Compute an exact small-position bit for an n-gram occurrence.
///
/// Bits 0..30 cover byte positions 0..30. Bit 31 means overflow/unknown and
/// must be treated as "cannot reject" by query-time filters.
#[inline]
pub fn zone_bit(position: usize) -> u32 {
    let zone = position / POSITION_ZONE_SIZE;
    if zone < POSITION_ZONE_COUNT {
        1u32 << zone
    } else {
        POSITION_ZONE_OVERFLOW_BIT
    }
}

/// Hash an n-gram (variable-length byte slice) into a NgramKey.
/// Uses FNV-1a for good distribution and speed.
pub fn hash_ngram(bytes: &[u8]) -> NgramKey {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Corpus-tuned bigram document frequency table.
/// Loaded from `.ig/bigram_df.bin`. Enables IDF-weighted ngram boundary selection.
pub struct BigramDfTable {
    entries: Vec<(u32, u32)>, // sorted by hash
    total_docs: u32,
}

impl BigramDfTable {
    #[allow(dead_code)]
    pub fn new(entries: Vec<(u32, u32)>, total_docs: u32) -> Self {
        Self {
            entries,
            total_docs,
        }
    }

    /// Load from .ig/bigram_df.bin
    pub fn load(ig_dir: &Path) -> Option<Self> {
        let path = ig_dir.join("bigram_df.bin");
        let bytes = std::fs::read(&path).ok()?;
        let entries: Vec<(u32, u32)> = bincode::deserialize(&bytes).ok()?;
        let meta = IndexMetadata::load_from(ig_dir).ok()?;
        Some(Self {
            entries,
            total_docs: meta.file_count,
        })
    }

    /// Look up document frequency for a bigram hash. Returns 0 if not found.
    pub fn df(&self, bigram_hash: u32) -> u32 {
        match self.entries.binary_search_by_key(&bigram_hash, |&(h, _)| h) {
            Ok(idx) => self.entries[idx].1,
            Err(_) => 0,
        }
    }

    /// IDF multiplier for a bigram. Range: 0.25 (very common) to 3.0 (very rare).
    pub fn idf_multiplier(&self, bigram_hash: u32) -> f32 {
        if self.total_docs == 0 {
            return 1.0;
        }
        let df = self.df(bigram_hash) as f32;
        let ratio = df / self.total_docs as f32;
        if ratio > 0.50 {
            0.25 // very common → push boundary away
        } else if ratio > 0.20 {
            0.50 // common
        } else if ratio < 0.01 {
            3.0 // very rare → attract boundary
        } else if ratio < 0.05 {
            2.0 // rare
        } else {
            1.0 // neutral
        }
    }
}

/// IDF-aware bigram weight. Falls back to heuristic when no DF table available.
#[inline]
pub(crate) fn bigram_weight_idf(a: u8, b: u8, df_table: Option<&BigramDfTable>) -> u32 {
    if let Some(df) = df_table {
        let base_hash = hash_bigram(a, b);
        let idf = df.idf_multiplier(base_hash);
        let heuristic = bigram_weight(a, b);
        return ((heuristic as f32 * idf) as u32).max(1);
    }

    bigram_weight(a, b) // fallback: existing heuristics
}

struct HashAndPos {
    hash: u32,
    pos: usize,
}

/// Build ALL sparse n-grams from a byte slice.
/// Uses a monotonic stack. Produces at most 2n-2 n-grams.
/// Returns (start, end) byte ranges into the input slice.
///
/// When `df_table` is `Some`, bigram weights are IDF-adjusted so that
/// boundaries favour rare corpus bigrams over common ones.
pub fn build_all_ngrams(data: &[u8], df_table: Option<&BigramDfTable>) -> Vec<(usize, usize)> {
    if data.len() < 3 {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut stack: Vec<HashAndPos> = Vec::new();

    for i in 0..data.len() - 1 {
        let p = HashAndPos {
            hash: bigram_weight_idf(data[i], data[i + 1], df_table),
            pos: i,
        };

        // Phase 1: pop smaller hashes
        while !stack.is_empty() && p.hash > stack.last().unwrap().hash {
            result.push((stack.last().unwrap().pos, i + 2));

            // Glue same hashes to the left
            while stack.len() > 1 && stack.last().unwrap().hash == stack[stack.len() - 2].hash {
                stack.pop();
            }
            stack.pop();
        }

        // Phase 2: emit from current top (if any) before push
        if !stack.is_empty() {
            result.push((stack.last().unwrap().pos, i + 2));
        }

        stack.push(p);
    }

    result
}

/// Build COVERING sparse n-grams from a byte slice.
/// Uses a deque. Produces at most n-2 n-grams (minimal covering set).
/// Returns (start, end) byte ranges into the input slice.
///
/// When `df_table` is `Some`, bigram weights are IDF-adjusted so that
/// boundaries favour rare corpus bigrams over common ones.
pub fn build_covering_ngrams(
    data: &[u8],
    max_ngram_len: usize,
    df_table: Option<&BigramDfTable>,
) -> Vec<(usize, usize)> {
    if data.len() < 3 {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut deque: VecDeque<HashAndPos> = VecDeque::new();

    for i in 0..data.len() - 1 {
        let p = HashAndPos {
            hash: bigram_weight_idf(data[i], data[i + 1], df_table),
            pos: i,
        };

        // Phase 1: window enforcement — flush front if span too long
        if deque.len() > 1 && (i + 3).saturating_sub(deque.front().unwrap().pos) >= max_ngram_len {
            let front_pos = deque.front().unwrap().pos;
            let second_pos = deque[1].pos;
            result.push((front_pos, second_pos + 2));
            deque.pop_front();
        }

        // Phase 2: maintain monotonic stack property (pop smaller from back)
        while !deque.is_empty() && p.hash > deque.back().unwrap().hash {
            // Special case: front.hash == back.hash (all remaining equal)
            if deque.front().unwrap().hash == deque.back().unwrap().hash {
                result.push((deque.back().unwrap().pos, i + 2));
                // Drain remaining
                while deque.len() > 1 {
                    let last_pos = deque.back().unwrap().pos + 2;
                    deque.pop_back();
                    result.push((deque.back().unwrap().pos, last_pos));
                }
            }
            deque.pop_back();
        }

        deque.push_back(p);
    }

    // Drain remaining entries from back
    while deque.len() > 1 {
        let last_pos = deque.back().unwrap().pos + 2;
        deque.pop_back();
        result.push((deque.back().unwrap().pos, last_pos));
    }

    result
}

/// Extract unique NgramKeys from a byte slice using sparse n-grams.
/// This is the main entry point for indexing.
/// Includes explicit bigram keys so that 2-character queries can use the index
/// instead of falling back to brute-force scan.
///
/// When `df_table` is `Some`, IDF-weighted boundary selection produces more
/// selective n-grams, reducing false positives at query time.
#[allow(dead_code)]
pub fn extract_sparse_ngrams(data: &[u8], df_table: Option<&BigramDfTable>) -> Vec<NgramKey> {
    let ranges = build_all_ngrams(data, df_table);
    let mut keys: Vec<NgramKey> = ranges
        .iter()
        .map(|&(start, end)| hash_ngram(&data[start..end]))
        .collect();

    // Add bigram keys for every consecutive byte pair.
    // This enables index-based lookup for 2-character patterns.
    if data.len() >= 2 {
        for window in data.windows(2) {
            keys.push(hash_ngram(window));
        }
    }

    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Extract unique NgramKeys with bloom and loc masks from a byte slice.
/// Returns `Vec<(NgramKey, bloom_mask, loc_mask)>` where masks are OR'd across
/// all occurrences in the file.
///
/// - bloom_mask: for each ngram occurrence at range [start..end), if data[end] exists,
///   set `bloom_bit(data[end])`. Enables "3.5-gram" filtering at query time.
/// - loc_mask: for each ngram occurrence at position start, set `loc_bit(start)`.
///   Enables adjacency filtering at query time.
pub fn extract_sparse_ngrams_with_masks(
    data: &[u8],
    df_table: Option<&BigramDfTable>,
) -> Vec<(NgramKey, u8, u8, u32)> {
    let ranges = build_all_ngrams(data, df_table);
    let mut mask_map: AHashMap<NgramKey, (u8, u8, u32)> = AHashMap::new();

    for &(start, end) in &ranges {
        let key = hash_ngram(&data[start..end]);
        let entry = mask_map.entry(key).or_insert((0u8, 0u8, 0u32));
        // bloom: hash the follow character (byte after the ngram)
        if end < data.len() {
            entry.0 |= bloom_bit(data[end]);
        }
        // loc: position of this ngram occurrence
        entry.1 |= loc_bit(start);
        // zone: exact small-position mask, with overflow for later bytes
        entry.2 |= zone_bit(start);
    }

    // Also add bigram keys with masks
    if data.len() >= 2 {
        for (i, window) in data.windows(2).enumerate() {
            let key = hash_ngram(window);
            let entry = mask_map.entry(key).or_insert((0u8, 0u8, 0u32));
            // bloom: follow char after the bigram
            if i + 2 < data.len() {
                entry.0 |= bloom_bit(data[i + 2]);
            }
            // loc: position of this bigram
            entry.1 |= loc_bit(i);
            entry.2 |= zone_bit(i);
        }
    }

    let mut result: Vec<(NgramKey, u8, u8, u32)> = mask_map
        .into_iter()
        .map(|(key, (bloom, loc, zone))| (key, bloom, loc, zone))
        .collect();
    result.sort_unstable_by_key(|(k, _, _, _)| *k);
    result.dedup_by_key(|(k, _, _, _)| *k);
    result
}

/// Extract covering NgramKeys from a byte slice (for queries).
///
/// When `df_table` is `Some`, IDF-weighted boundary selection produces
/// covering n-grams that better match the index-time boundaries.
#[allow(dead_code)]
pub fn extract_covering_ngrams(
    data: &[u8],
    max_len: usize,
    df_table: Option<&BigramDfTable>,
) -> Vec<NgramKey> {
    let ranges = build_covering_ngrams(data, max_len, df_table);
    let mut keys: Vec<NgramKey> = ranges
        .iter()
        .map(|&(start, end)| hash_ngram(&data[start..end]))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_ngram_strings(data: &[u8], ranges: &[(usize, usize)]) -> Vec<String> {
        ranges
            .iter()
            .map(|&(s, e)| String::from_utf8_lossy(&data[s..e]).to_string())
            .collect()
    }

    #[test]
    fn test_hash_bigram_deterministic() {
        let h1 = hash_bigram(b'h', b'e');
        let h2 = hash_bigram(b'h', b'e');
        assert_eq!(h1, h2);
        // Different bigrams should (usually) give different hashes
        let h3 = hash_bigram(b'e', b'l');
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_build_all_empty_and_short() {
        assert!(build_all_ngrams(b"", None).is_empty());
        assert!(build_all_ngrams(b"h", None).is_empty());
        assert!(build_all_ngrams(b"he", None).is_empty());
    }

    #[test]
    fn test_build_all_three_chars() {
        let data = b"hel";
        let ranges = build_all_ngrams(data, None);
        let strings = collect_ngram_strings(data, &ranges);
        assert!(strings.contains(&"hel".to_string()));
    }

    #[test]
    fn test_build_covering_hello_world() {
        let data = b"hello world";
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN, None);
        let strings = collect_ngram_strings(data, &ranges);
        // Expected from danlark1: {"hel","ell","llo","rld","worl","lo wo"}
        assert_eq!(
            strings.len(),
            6,
            "expected 6 covering ngrams, got {:?}",
            strings
        );
        assert!(strings.contains(&"hel".to_string()));
        assert!(strings.contains(&"rld".to_string()));
    }

    #[test]
    fn test_build_covering_chester() {
        let data = b"chester ";
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN, None);
        let strings = collect_ngram_strings(data, &ranges);
        // With corpus-tuned weights (WEIGHT_VERSION 2), "er " gets a penalty
        // (common bigram "er" + space-lower), shifting boundaries:
        // {"chest","er ","ter","ste"}
        assert_eq!(
            strings.len(),
            4,
            "expected 4 covering ngrams, got {:?}",
            strings
        );
        assert!(strings.contains(&"chest".to_string()));
        assert!(strings.contains(&"er ".to_string()));
    }

    #[test]
    fn test_build_covering_code() {
        let data = b"for(int i=42";
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN, None);
        let strings = collect_ngram_strings(data, &ranges);
        // Expected from danlark1: {"for(i","(int i=4","=42"}
        assert_eq!(
            strings.len(),
            3,
            "expected 3 covering ngrams, got {:?}",
            strings
        );
        assert!(strings.contains(&"=42".to_string()));
    }

    #[test]
    fn test_extract_sparse_ngrams() {
        let data = b"hello world";
        let keys = extract_sparse_ngrams(data, None);
        assert!(!keys.is_empty());
        // Keys should be sorted and deduped
        for w in keys.windows(2) {
            assert!(w[0] < w[1], "keys should be sorted and unique");
        }
    }

    #[test]
    fn test_hash_ngram_different_inputs() {
        assert_ne!(hash_ngram(b"hello"), hash_ngram(b"world"));
        assert_eq!(hash_ngram(b"test"), hash_ngram(b"test"));
    }

    #[test]
    fn test_bigram_weight_deterministic() {
        let w1 = bigram_weight(b'h', b'e');
        let w2 = bigram_weight(b'h', b'e');
        assert_eq!(w1, w2, "bigram_weight must be deterministic");
    }

    #[test]
    fn test_bigram_weight_camel_case_boost() {
        // camelCase boundary: lowercase → uppercase gets 2x boost
        let base = hash_bigram(b'm', b'N');
        let weighted = bigram_weight(b'm', b'N');
        assert_eq!(
            weighted,
            ((base as u64).wrapping_mul(2) & 0xFFFF_FFFF) as u32
        );
    }

    #[test]
    fn test_bigram_weight_snake_case_boost() {
        // snake_case boundary: lowercase → underscore gets 2x boost
        let base = hash_bigram(b'x', b'_');
        let weighted = bigram_weight(b'x', b'_');
        assert_eq!(
            weighted,
            ((base as u64).wrapping_mul(2) & 0xFFFF_FFFF) as u32
        );
    }

    #[test]
    fn test_bigram_weight_structural_boost() {
        // Structural delimiters get 1.5x boost
        let base = hash_bigram(b'f', b'(');
        let weighted = bigram_weight(b'f', b'(');
        assert_eq!(
            weighted,
            ((base as u64).wrapping_mul(3).wrapping_div(2) & 0xFFFF_FFFF) as u32
        );
    }

    #[test]
    fn test_bigram_weight_common_pair_penalty() {
        // Common bigram "he" gets 0.5x penalty
        let base = hash_bigram(b'h', b'e');
        let weighted = bigram_weight(b'h', b'e');
        assert_eq!(weighted, (base as u64).wrapping_div(2) as u32);
    }

    #[test]
    fn test_bigram_weight_space_lower_penalty() {
        // space + lowercase gets 0.5x penalty
        let base = hash_bigram(b' ', b'a');
        let weighted = bigram_weight(b' ', b'a');
        assert_eq!(weighted, (base as u64).wrapping_div(2) as u32);
    }

    #[test]
    fn test_bigram_weight_neutral_unchanged() {
        // Plain lowercase pair not in the common list → no adjustment
        let base = hash_bigram(b'x', b'y');
        let weighted = bigram_weight(b'x', b'y');
        assert_eq!(weighted, base, "neutral pair should keep base weight");
    }

    #[test]
    fn test_idf_multiplier() {
        // 100 total docs
        let entries = vec![
            (10, 60), // hash 10 → 60/100 = 0.60 → very common → 0.25
            (20, 30), // hash 20 → 30/100 = 0.30 → common → 0.50
            (30, 10), // hash 30 → 10/100 = 0.10 → neutral → 1.0
            (40, 3),  // hash 40 → 3/100 = 0.03 → rare → 2.0
            (50, 0),  // hash 50 → 0/100 = 0.00 → very rare → 3.0
        ];
        let table = BigramDfTable::new(entries, 100);

        assert_eq!(table.idf_multiplier(10), 0.25);
        assert_eq!(table.idf_multiplier(20), 0.50);
        assert_eq!(table.idf_multiplier(30), 1.0);
        assert_eq!(table.idf_multiplier(40), 2.0);
        assert_eq!(table.idf_multiplier(50), 3.0);

        // Unknown hash → df=0 → very rare → 3.0
        assert_eq!(table.idf_multiplier(999), 3.0);

        // Zero total docs → always 1.0
        let empty_table = BigramDfTable::new(vec![], 0);
        assert_eq!(empty_table.idf_multiplier(10), 1.0);
    }

    #[test]
    fn test_bigram_weight_idf_with_table() {
        // Build a table where the bigram hash for ('x', 'y') is very common
        let xy_hash = hash_bigram(b'x', b'y');
        let entries = vec![(xy_hash, 80)]; // 80/100 = 0.80 → very common → 0.25
        let table = BigramDfTable::new(entries, 100);

        let without_idf = bigram_weight(b'x', b'y');
        let with_idf = bigram_weight_idf(b'x', b'y', Some(&table));

        // IDF=0.25 for very common bigram: weight should be ~1/4 of heuristic
        assert!(
            with_idf < without_idf,
            "IDF should reduce weight for common bigram: {} vs {}",
            with_idf,
            without_idf
        );
        assert_eq!(with_idf, ((without_idf as f32 * 0.25) as u32).max(1));

        // Without table: identical to heuristic
        let fallback = bigram_weight_idf(b'x', b'y', None);
        assert_eq!(fallback, without_idf);
    }

    #[test]
    fn test_covering_with_idf_produces_different_ngrams() {
        // Build a DF table that marks certain bigrams as very common/rare,
        // which should shift boundary placement compared to heuristic-only.
        let data = b"hello_world_test";

        // Collect all bigram hashes from the data
        let mut entries = Vec::new();
        for w in data.windows(2) {
            let h = hash_bigram(w[0], w[1]);
            // Mark "lo" and "or" as very common (push boundaries away)
            if (w[0], w[1]) == (b'l', b'o') || (w[0], w[1]) == (b'o', b'r') {
                entries.push((h, 90)); // 90% of docs
            }
            // Mark "te" as very rare (attract boundaries)
            else if (w[0], w[1]) == (b't', b'e') {
                entries.push((h, 0)); // 0% of docs
            }
        }
        entries.sort_unstable_by_key(|&(h, _)| h);
        entries.dedup_by_key(|e| e.0);
        let table = BigramDfTable::new(entries, 100);

        let ranges_without = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN, None);
        let ranges_with = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN, Some(&table));

        // The IDF weighting should produce different boundary placements
        // (different set of ngram ranges) for at least some inputs.
        // We compare the actual ranges — they should differ.
        assert_ne!(
            ranges_without, ranges_with,
            "IDF weighting should change boundary placement"
        );
    }

    #[test]
    fn test_bloom_bit() {
        // Each unique input should set exactly one bit
        for ch in 0u8..=255 {
            let bit = bloom_bit(ch);
            assert!(bit.is_power_of_two(), "bloom_bit must set exactly one bit");
        }
        // Deterministic
        assert_eq!(bloom_bit(b'd'), bloom_bit(b'd'));
        // Different chars can map to different bits (not guaranteed but likely)
        // At minimum, verify the function doesn't always return the same value
        let mut seen = std::collections::HashSet::new();
        for ch in b'a'..=b'z' {
            seen.insert(bloom_bit(ch));
        }
        assert!(seen.len() > 1, "bloom_bit should vary across characters");
    }

    #[test]
    fn test_loc_bit() {
        // Position modulo 8 determines the bit
        for pos in 0..8 {
            assert_eq!(loc_bit(pos), 1u8 << pos);
        }
        // Wraps at 8
        assert_eq!(loc_bit(0), loc_bit(8));
        assert_eq!(loc_bit(1), loc_bit(9));
    }

    #[test]
    fn test_extract_sparse_ngrams_with_masks() {
        let data = b"hello world";
        let results = extract_sparse_ngrams_with_masks(data, None);
        assert!(!results.is_empty());

        // Keys should be sorted and unique
        for w in results.windows(2) {
            assert!(w[0].0 < w[1].0, "keys should be sorted and unique");
        }

        // Every entry should have some mask bits set (for non-trivial data)
        let has_bloom = results.iter().any(|(_, bloom, _, _)| *bloom != 0);
        let has_loc = results.iter().any(|(_, _, loc, _)| *loc != 0);
        let has_zone = results.iter().any(|(_, _, _, zone)| *zone != 0);
        assert!(has_bloom, "some ngrams should have bloom bits set");
        assert!(has_loc, "some ngrams should have loc bits set");
        assert!(has_zone, "some ngrams should have zone bits set");

        // The set of keys should match extract_sparse_ngrams (same keys, just with masks)
        let keys_with_masks: Vec<NgramKey> = results.iter().map(|(k, _, _, _)| *k).collect();
        let keys_plain = extract_sparse_ngrams(data, None);
        assert_eq!(keys_with_masks, keys_plain, "same key set expected");
    }
}
