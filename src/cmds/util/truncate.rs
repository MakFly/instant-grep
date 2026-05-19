//! Char-boundary-safe slice truncation. Mirrors `str::floor_char_boundary`
//! (stabilised in 1.83) but stays available for any future MSRV drift.

/// Return a prefix of `s` no longer than `max` bytes that ends on a valid
/// UTF-8 char boundary. `s` is returned unchanged when `s.len() <= max`.
pub fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_truncate_when_under() {
        assert_eq!(truncate_at_char_boundary("hi", 5), "hi");
    }

    #[test]
    fn truncates_ascii() {
        assert_eq!(truncate_at_char_boundary("abcdef", 3), "abc");
    }

    #[test]
    fn never_splits_codepoint() {
        // `é` is two bytes in UTF-8. Asking for 1 byte must back off to 0.
        assert_eq!(truncate_at_char_boundary("é", 1), "");
    }

    #[test]
    fn truncates_unicode() {
        let s = "café";
        // 'café' is 5 bytes; cut at 4 lands on the boundary right before 'é'
        // (actually inside 'é', so we back off to 3 = "caf").
        let out = truncate_at_char_boundary(s, 4);
        assert_eq!(out, "caf");
    }
}
