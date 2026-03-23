use std::collections::VecDeque;

/// Hash of a variable-length n-gram, used as index key.
pub type NgramKey = u64;

/// Default max n-gram length for covering algorithm.
pub const DEFAULT_MAX_NGRAM_LEN: usize = 16;

/// Hash a bigram (2 consecutive bytes) using Murmur2-like constants.
/// Port of danlark1/sparse_ngrams HashBigram.
#[inline]
fn hash_bigram(a: u8, b: u8) -> u32 {
    const MUL1: u64 = 0xc6a4a7935bd1e995;
    const MUL2: u64 = 0x228876a7198b743;
    let v = (a as u64)
        .wrapping_mul(MUL1)
        .wrapping_add((b as u64).wrapping_mul(MUL2));
    (v.wrapping_add(!v >> 47)) as u32
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

struct HashAndPos {
    hash: u32,
    pos: usize,
}

/// Build ALL sparse n-grams from a byte slice.
/// Uses a monotonic stack. Produces at most 2n-2 n-grams.
/// Returns (start, end) byte ranges into the input slice.
pub fn build_all_ngrams(data: &[u8]) -> Vec<(usize, usize)> {
    if data.len() < 3 {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut stack: Vec<HashAndPos> = Vec::new();

    for i in 0..data.len() - 1 {
        let p = HashAndPos {
            hash: hash_bigram(data[i], data[i + 1]),
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
pub fn build_covering_ngrams(data: &[u8], max_ngram_len: usize) -> Vec<(usize, usize)> {
    if data.len() < 3 {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut deque: VecDeque<HashAndPos> = VecDeque::new();

    for i in 0..data.len() - 1 {
        let p = HashAndPos {
            hash: hash_bigram(data[i], data[i + 1]),
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
pub fn extract_sparse_ngrams(data: &[u8]) -> Vec<NgramKey> {
    let ranges = build_all_ngrams(data);
    let mut keys: Vec<NgramKey> = ranges
        .iter()
        .map(|&(start, end)| hash_ngram(&data[start..end]))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Extract covering NgramKeys from a byte slice (for queries).
pub fn extract_covering_ngrams(data: &[u8], max_len: usize) -> Vec<NgramKey> {
    let ranges = build_covering_ngrams(data, max_len);
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
        assert!(build_all_ngrams(b"").is_empty());
        assert!(build_all_ngrams(b"h").is_empty());
        assert!(build_all_ngrams(b"he").is_empty());
    }

    #[test]
    fn test_build_all_three_chars() {
        let data = b"hel";
        let ranges = build_all_ngrams(data);
        let strings = collect_ngram_strings(data, &ranges);
        assert!(strings.contains(&"hel".to_string()));
    }

    #[test]
    fn test_build_covering_hello_world() {
        let data = b"hello world";
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN);
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
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN);
        let strings = collect_ngram_strings(data, &ranges);
        // Expected from danlark1: {"chest","ster","er "}
        assert_eq!(
            strings.len(),
            3,
            "expected 3 covering ngrams, got {:?}",
            strings
        );
        assert!(strings.contains(&"chest".to_string()));
        assert!(strings.contains(&"ster".to_string()));
        assert!(strings.contains(&"er ".to_string()));
    }

    #[test]
    fn test_build_covering_code() {
        let data = b"for(int i=42";
        let ranges = build_covering_ngrams(data, DEFAULT_MAX_NGRAM_LEN);
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
        let keys = extract_sparse_ngrams(data);
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
}
