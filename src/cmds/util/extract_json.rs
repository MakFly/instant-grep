//! Robust JSON extraction from noisy stdout (banner lines, dotenv prefixes, …).
//!
//! Used by every JSON-reporter parser (vitest, jest, eslint, rubocop, …) so
//! that a wrapper like `pnpm exec vitest` that prints
//! `> my-app@0.0.1 test\n> vitest --reporter=json\n…\n{ ... }` still parses.

/// Find the largest balanced `{...}` object in `stdout`, returning a slice
/// into the original string. Returns `None` if no balanced object is found.
///
/// Behaviour:
/// - Skips any leading text before the first `{`.
/// - Tracks string literals (so `{` inside `"..."` doesn't count).
/// - Handles backslash escapes inside strings (`"\""`).
/// - If multiple top-level objects exist, returns the longest one. This is
///   what JSON test reporters that emit a single result object always satisfy.
pub fn extract_json_object(stdout: &str) -> Option<&str> {
    let bytes = stdout.as_bytes();
    let mut best: Option<(usize, usize)> = None; // (start, end_exclusive)
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{'
            && let Some(end) = scan_balanced(bytes, i)
        {
            let len = end - i;
            if best.map(|(s, e)| (e - s) < len).unwrap_or(true) {
                best = Some((i, end));
            }
            i = end;
            continue;
        }
        i += 1;
    }
    best.map(|(s, e)| &stdout[s..e])
}

/// Return an iterator over balanced JSON lines (NDJSON). Each item is one
/// line that successfully starts with `{` and ends with `}`. Lines failing
/// this trivial check are skipped (typical for `go test -json` where
/// non-JSON stderr can be interleaved).
pub fn extract_json_lines(stdout: &str) -> impl Iterator<Item = &str> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{') && l.ends_with('}'))
}

/// Walk a balanced `{...}` starting at byte index `start` (which must point
/// at a `{`). Returns the exclusive end index of the matching `}`, or `None`
/// if the input ends before the object closes.
fn scan_balanced(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes[start], b'{');
    let mut depth: usize = 0;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_object() {
        let s = r#"prefix noise
{"a":1}
trailing"#;
        assert_eq!(extract_json_object(s), Some(r#"{"a":1}"#));
    }

    #[test]
    fn handles_dotenv_prefix() {
        let s = "[dotenv] loaded .env\n> my-app@0.0.1 test\n> vitest --reporter=json\n\n{\"numTotalTests\":3,\"numPassedTests\":2}\n";
        let extracted = extract_json_object(s).unwrap();
        assert!(extracted.starts_with('{'));
        assert!(extracted.ends_with('}'));
        assert!(extracted.contains("numTotalTests"));
    }

    #[test]
    fn skips_braces_in_strings() {
        let s = r#"junk {"msg":"a } b","ok":true} done"#;
        let out = extract_json_object(s).unwrap();
        assert_eq!(out, r#"{"msg":"a } b","ok":true}"#);
    }

    #[test]
    fn picks_largest_when_multiple() {
        let s = r#"a {"x":1} b {"x":1,"y":{"z":2}} c"#;
        let out = extract_json_object(s).unwrap();
        assert!(out.contains("\"y\":"));
    }

    #[test]
    fn returns_none_on_unbalanced() {
        let s = r#"prefix {"a":1"#;
        assert_eq!(extract_json_object(s), None);
    }

    #[test]
    fn ndjson_iterates_lines() {
        let s = "{\"a\":1}\nnoise\n{\"b\":2}\n{not json}";
        let lines: Vec<_> = extract_json_lines(s).collect();
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}", "{not json}"]);
    }

    #[test]
    fn escape_inside_string() {
        let s = r#"x {"k":"a\"}b"} y"#;
        let out = extract_json_object(s).unwrap();
        assert_eq!(out, r#"{"k":"a\"}b"}"#);
    }
}
