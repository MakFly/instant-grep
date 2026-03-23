pub type Trigram = u32;

/// Pack 3 bytes into a u32 trigram (lower 24 bits).
#[inline]
pub fn pack_trigram(a: u8, b: u8, c: u8) -> Trigram {
    ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

/// Extract all unique trigrams from a byte slice. Returns sorted, deduplicated vec.
pub fn extract_trigrams(data: &[u8]) -> Vec<Trigram> {
    if data.len() < 3 {
        return Vec::new();
    }

    let mut trigrams = Vec::with_capacity(data.len() - 2);
    for window in data.windows(3) {
        trigrams.push(pack_trigram(window[0], window[1], window[2]));
    }

    trigrams.sort_unstable();
    trigrams.dedup();
    trigrams
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_trigram() {
        let t = pack_trigram(b'f', b'u', b'n');
        assert_eq!(t, (b'f' as u32) << 16 | (b'u' as u32) << 8 | b'n' as u32);
    }

    #[test]
    fn test_extract_trigrams_short() {
        assert!(extract_trigrams(b"").is_empty());
        assert!(extract_trigrams(b"ab").is_empty());
    }

    #[test]
    fn test_extract_trigrams_function() {
        let trigrams = extract_trigrams(b"function");
        // "function" -> fun, unc, nct, cti, tio, ion = 6 unique trigrams
        assert_eq!(trigrams.len(), 6);
        assert!(trigrams.contains(&pack_trigram(b'f', b'u', b'n')));
        assert!(trigrams.contains(&pack_trigram(b'i', b'o', b'n')));
    }

    #[test]
    fn test_extract_trigrams_dedup() {
        // "aaa" has only one unique trigram
        let trigrams = extract_trigrams(b"aaaa");
        assert_eq!(trigrams.len(), 1);
        assert_eq!(trigrams[0], pack_trigram(b'a', b'a', b'a'));
    }
}
