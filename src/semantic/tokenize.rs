//! Identifier-aware tokenizer.
//!
//! Splits on:
//!   - non-alphanumeric characters (punctuation, whitespace, symbols)
//!   - camelCase boundaries:           `handleSubmit`      → [handle, submit]
//!   - snake_case underscores:         `get_user_by_id`    → [get, user, by, id]
//!   - kebab-case hyphens:             `data-testid`       → [data, testid]
//!   - UPPER acronym → lower word:     `HTTPRequest`       → [http, request]
//!   - UPPER_SNAKE:                    `MAX_RETRY_COUNT`   → [max, retry, count]
//!
//! All output tokens are lowercase ASCII. Tokens shorter than 2 chars and
//! the handful of keywords that dominate any codebase (and contribute no
//! semantic signal) are dropped.
//!
//! Unicode: we operate on chars; non-ASCII letters are kept as lowercase.
//! Non-letter-or-digit chars are always separators.

use std::collections::HashSet;

/// Tokens shorter than this are dropped.
const MIN_LEN: usize = 2;

/// Hard-coded stop list. These appear in essentially every source file and
/// carry no semantic signal worth learning co-occurrences for.
const STOPWORDS: &[&str] = &[
    // English prepositions + articles + pronouns
    "the", "and", "but", "not", "you", "are", "with", "this", "that",
    "from", "into", "off", "to", "of", "in", "is", "it", "be", "as",
    "on", "by", "or", "an", "at", "so", "we", "he", "she", "his", "her",
    "has", "was", "any", "all",
    // Generic language keywords with no semantic weight for search
    "let", "var", "def", "else", "then", "do", "end",
    "true", "false", "null", "nil", "none", "void",
    // NB: we deliberately keep "use", "get", "set", "for", "if", "fn",
    // "const", "return", "throw", "new" — they carry intent signal
    // (React `useState`, error-handling `throw`, immutability `const`, …)
];

pub struct Tokenizer {
    stopwords: HashSet<&'static str>,
}

impl Tokenizer {
    pub fn new() -> Self {
        Self {
            stopwords: STOPWORDS.iter().copied().collect(),
        }
    }

    /// Tokenize one line of source code. Allocates a new Vec.
    /// For hot loops, prefer `tokenize_into`.
    pub fn tokenize(&self, line: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.tokenize_into(line, &mut out);
        out
    }

    /// Tokenize into a caller-provided buffer. Clears the buffer first.
    pub fn tokenize_into(&self, line: &str, out: &mut Vec<String>) {
        out.clear();
        for raw in line.split(|c: char| !c.is_alphanumeric()) {
            if raw.is_empty() {
                continue;
            }
            split_identifier(raw, &mut |piece| {
                if piece.len() < MIN_LEN {
                    return;
                }
                let lower = piece.to_lowercase();
                if self.stopwords.contains(lower.as_str()) {
                    return;
                }
                // Ignore purely numeric tokens
                if lower.chars().all(|c| c.is_ascii_digit()) {
                    return;
                }
                // Ignore JSON unicode-escape artefacts: `u` + 4 hex digits
                // (e.g. "u00e9lectionner" = JSON-encoded "sélectionner").
                if is_unicode_escape_artefact(&lower) {
                    return;
                }
                out.push(lower);
            });
        }
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Split an already-separator-free word into camelCase / snake_case pieces.
/// Calls `emit` on each piece in order.
fn split_identifier(word: &str, emit: &mut dyn FnMut(&str)) {
    // Fast path: pure lowercase or pure uppercase — no split needed.
    if word.chars().all(|c| !c.is_alphabetic() || c.is_lowercase())
        || word.chars().all(|c| !c.is_alphabetic() || c.is_uppercase())
    {
        emit(word);
        return;
    }

    let chars: Vec<char> = word.chars().collect();
    let len = chars.len();
    let mut start = 0;

    let mut i = 0;
    while i < len {
        let c = chars[i];
        let is_upper = c.is_uppercase();
        let prev_lower = i > 0 && chars[i - 1].is_lowercase();
        let next_lower = i + 1 < len && chars[i + 1].is_lowercase();

        // Boundary: lower → Upper  (userName → user | Name)
        if is_upper && prev_lower {
            emit_slice(&chars, start, i, emit);
            start = i;
        }
        // Boundary: UPPER → UpperLower  (HTTPRequest → HTTP | Request)
        else if is_upper && i > start + 1 && next_lower {
            emit_slice(&chars, start, i, emit);
            start = i;
        }

        i += 1;
    }
    emit_slice(&chars, start, len, emit);
}

/// Detect tokens that look like they come from a leaked JSON-escape sequence
/// such as `u00e9lectionner` (what `électionner` becomes after our
/// punctuation-based split). Matches `u` + exactly 4 hex digits at the start.
fn is_unicode_escape_artefact(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 5 {
        return false;
    }
    if bytes[0] != b'u' {
        return false;
    }
    bytes[1..5].iter().all(|b| b.is_ascii_hexdigit())
}

fn emit_slice(chars: &[char], start: usize, end: usize, emit: &mut dyn FnMut(&str)) {
    if start >= end {
        return;
    }
    let s: String = chars[start..end].iter().collect();
    if !s.is_empty() {
        emit(&s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(s: &str) -> Vec<String> {
        Tokenizer::new().tokenize(s)
    }

    #[test]
    fn plain_words() {
        assert_eq!(tokenize("hello world"), vec!["hello", "world"]);
    }

    #[test]
    fn camel_case() {
        assert_eq!(tokenize("handleSubmit"), vec!["handle", "submit"]);
        assert_eq!(
            tokenize("useEffectiveCallback"),
            vec!["use", "effective", "callback"]
        );
    }

    #[test]
    fn snake_case() {
        // "by" is a stop-word; "get"/"user"/"id" are kept.
        assert_eq!(
            tokenize("get_user_by_id"),
            vec!["get", "user", "id"]
        );
        assert_eq!(tokenize("my_function_name"), vec!["my", "function", "name"]);
    }

    #[test]
    fn kebab_case() {
        assert_eq!(tokenize("data-testid"), vec!["data", "testid"]);
    }

    #[test]
    fn upper_snake() {
        assert_eq!(tokenize("MAX_RETRY_COUNT"), vec!["max", "retry", "count"]);
    }

    #[test]
    fn acronym_then_word() {
        // HTTPRequest → HTTP + Request
        assert_eq!(tokenize("HTTPRequest"), vec!["http", "request"]);
        assert_eq!(tokenize("IOError"), vec!["io", "error"]);
        assert_eq!(tokenize("parseURL"), vec!["parse", "url"]);
    }

    #[test]
    fn drops_short_tokens() {
        // "a", "b", "I" are all < MIN_LEN → dropped
        assert_eq!(tokenize("a b c I"), Vec::<String>::new());
    }

    #[test]
    fn drops_stopwords() {
        // "the" and "and" are stop-words; "fn" (kept intentionally) + "main" survive.
        assert_eq!(tokenize("fn main() { the and }"), vec!["fn", "main"]);
    }

    #[test]
    fn mixed_line_throw_catch() {
        // We keep "try"/"throw"/"catch"-adjacent tokens — they carry the
        // semantic signal we rely on for query expansion.
        let toks = tokenize(r#"try { throw new HttpException("boom"); }"#);
        assert!(toks.contains(&"throw".to_string()));
        assert!(toks.contains(&"http".to_string()));
        assert!(toks.contains(&"exception".to_string()));
        assert!(toks.contains(&"boom".to_string()));
    }

    #[test]
    fn drops_pure_numbers() {
        assert_eq!(tokenize("42 3.14 foo"), vec!["foo"]);
    }

    #[test]
    fn punctuation_is_separator() {
        assert_eq!(
            tokenize("foo.bar::baz/qux"),
            vec!["foo", "bar", "baz", "qux"]
        );
    }

    #[test]
    fn drops_json_unicode_escape_artefact() {
        // "sélectionner" leaks into JSON as `électionner`; our
        // punctuation-based split strips the backslash and we refuse the
        // `u00e9lectionner` that remains.
        assert_eq!(tokenize("u00e9lectionner"), Vec::<String>::new());
        // But genuine non-ASCII letters pass through normally.
        let toks = tokenize("sélectionner");
        assert!(toks.iter().any(|t| t.contains("sélectionner")));
    }

    #[test]
    fn unicode_letter_passthrough() {
        // Accented chars are letters → stay in the same token (lowercased).
        let toks = tokenize("café_crème");
        assert!(toks.iter().any(|t| t.contains("café")));
    }
}
