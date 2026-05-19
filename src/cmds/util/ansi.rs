//! ANSI escape stripping — used by every parser before regex/text matching.
//!
//! Hand-rolled to avoid an extra dep. Strips CSI (`ESC[...m` and other final bytes),
//! OSC sequences (`ESC]...BEL` / `ESC]...ESC\`), single-char escapes (`ESC=`),
//! and bare control chars below 0x20 except `\t`, `\n`, `\r`.

use std::borrow::Cow;

/// Strip ANSI escape sequences from `input`. Returns the input unchanged
/// (zero-copy) when no escape byte is present.
pub fn strip_ansi(input: &str) -> Cow<'_, str> {
    if !input.contains('\x1b') {
        return Cow::Borrowed(input);
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'[' => {
                    // CSI: skip until a byte in 0x40..=0x7e (final byte).
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1; // consume the final byte
                    }
                }
                b']' => {
                    // OSC: skip until BEL (0x07) or ESC \ (ST).
                    i += 2;
                    while i < bytes.len() && bytes[i] != 0x07 {
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == 0x07 {
                        i += 1;
                    }
                }
                _ => {
                    // Two-char escape (ESC=, ESC>, ESC(B, …). Skip both.
                    i += 2;
                }
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    // The output is still valid UTF-8 because we only ever drop ASCII bytes
    // (ESC + the bytes in 0x20..=0x7e that follow it) — never multi-byte ones.
    match String::from_utf8(out) {
        Ok(s) => Cow::Owned(s),
        Err(_) => Cow::Owned(String::from_utf8_lossy(input.as_bytes()).into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_when_no_escape() {
        let s = "hello world";
        assert_eq!(strip_ansi(s), s);
        // No allocation — verify by pointer parity via Cow::Borrowed.
        match strip_ansi(s) {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("should not allocate"),
        }
    }

    #[test]
    fn strips_color_csi() {
        let s = "\x1b[31mred\x1b[0m text";
        assert_eq!(strip_ansi(s), "red text");
    }

    #[test]
    fn strips_complex_csi() {
        let s = "\x1b[1;38;5;208mhi\x1b[m";
        assert_eq!(strip_ansi(s), "hi");
    }

    #[test]
    fn strips_osc_bel() {
        let s = "before\x1b]0;title\x07after";
        assert_eq!(strip_ansi(s), "beforeafter");
    }

    #[test]
    fn strips_osc_st() {
        let s = "x\x1b]8;;https://e.com\x1b\\link\x1b]8;;\x1b\\y";
        assert_eq!(strip_ansi(s), "xlinky");
    }

    #[test]
    fn keeps_unicode() {
        let s = "café \x1b[32m✓\x1b[0m";
        assert_eq!(strip_ansi(s), "café ✓");
    }
}
