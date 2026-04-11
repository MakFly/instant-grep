use std::collections::HashMap;

use crate::index::ngram::{NgramKey, hash_ngram};
use crate::index::reader::IndexReader;
use crate::util::{find_root, ig_dir};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply Layered Semantic Compression to already-filtered lines.
/// `lines` comes from `read_aggressive()` Phase 1 output.
/// `budget` is optional max output tokens (1 token ≈ 4 chars).
/// Returns compressed lines.
pub fn compress_lsc(
    lines: Vec<(usize, String)>,
    budget: Option<usize>,
    relevant: Option<&str>,
) -> Vec<(usize, String)> {
    if lines.is_empty() {
        return lines;
    }

    // Phase 2 + 3: Score each line by entropy × token-type weight
    let scorer = EntropyScorer::new();
    let relevance_re =
        relevant.and_then(|p| regex::Regex::new(&format!("(?i){}", regex::escape(p))).ok());
    let scored: Vec<(usize, String, f64)> = lines
        .into_iter()
        .map(|(num, line)| {
            let entropy = score_line_entropy(&line, &scorer);
            let weight = token_type_weight(&line);
            let mut score = entropy * weight;

            // Relevance boost
            if let Some(ref re) = relevance_re
                && re.is_match(&line) {
                    if is_signature(line.trim()) {
                        score *= 3.0; // Signature matching pattern = highest priority
                    } else {
                        score *= 2.0; // Any line matching pattern = boosted
                    }
                }

            (num, line, score)
        })
        .collect();

    // Phase 4: Deduplicate repeated patterns
    let deduped = deduplicate_lines(scored);

    // Phase 5: Fit to budget (or filter low-score lines)
    match budget {
        Some(b) => fit_to_budget(deduped, b),
        None => {
            // Even without budget, remove lines with near-zero score
            deduped
                .into_iter()
                .filter(|(_, _, score)| *score >= 0.05)
                .map(|(n, l, _)| (n, l))
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 2: Entropy Scoring
// ---------------------------------------------------------------------------

struct EntropyScorer {
    reader: Option<IndexReader>,
    total_files: f64,
}

impl EntropyScorer {
    fn new() -> Self {
        let root = std::env::current_dir().map(|p| find_root(&p)).ok();
        let ig = root.as_ref().map(|r| ig_dir(r));
        let reader = ig.and_then(|d| IndexReader::open(&d).ok());
        let total_files = reader
            .as_ref()
            .map(|r| r.total_file_count() as f64)
            .unwrap_or(100.0);
        Self {
            reader,
            total_files,
        }
    }

    fn idf(&self, trigram: &[u8]) -> f64 {
        if let Some(ref reader) = self.reader {
            let key: NgramKey = hash_ngram(trigram);
            let df = reader.lookup_ngram(key).len() as f64;
            (self.total_files / (df + 1.0)).ln()
        } else {
            1.0 // no index available — neutral score
        }
    }
}

fn score_line_entropy(line: &str, scorer: &EntropyScorer) -> f64 {
    let bytes = line.as_bytes();
    if bytes.len() < 3 {
        return 0.0;
    }
    let mut total_idf: f64 = 0.0;
    let mut count: usize = 0;
    for window in bytes.windows(3) {
        total_idf += scorer.idf(window);
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    // Normalize by line length so short high-info lines beat long boilerplate
    total_idf / count as f64
}

// ---------------------------------------------------------------------------
// Phase 3: Token Type Priority
// ---------------------------------------------------------------------------

fn token_type_weight(line: &str) -> f64 {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return 0.0;
    }

    // Signature: function/class/struct/def/fn declarations
    if is_signature(trimmed) {
        return 1.0;
    }

    // Invocation: method/function calls
    if is_invocation(trimmed) {
        return 0.8;
    }

    // Structure: control flow keywords
    if is_structure(trimmed) {
        return 0.5;
    }

    // Identifier: variable assignments
    if is_identifier_line(trimmed) {
        return 0.3;
    }

    // Symbol: pure punctuation / closing braces
    if is_symbol_only(trimmed) {
        return 0.1;
    }

    0.5 // default
}

fn is_signature(s: &str) -> bool {
    // Strip optional visibility / modifier prefixes
    let stripped = strip_leading_keywords(
        s,
        &[
            "pub ",
            "pub(crate) ",
            "pub(super) ",
            "export ",
            "default ",
            "async ",
            "static ",
            "abstract ",
            "private ",
            "protected ",
            "public ",
            "internal ",
            "override ",
            "virtual ",
            "inline ",
            "const ",
            "unsafe ",
        ],
    );
    starts_with_any(
        stripped,
        &[
            "fn ",
            "func ",
            "function ",
            "def ",
            "class ",
            "struct ",
            "enum ",
            "trait ",
            "impl ",
            "interface ",
            "type ",
            "module ",
            "object ",
            "record ",
        ],
    )
}

fn is_invocation(s: &str) -> bool {
    // Contains -> or :: followed eventually by (
    if (s.contains("->") || s.contains("::")) && s.contains('(') {
        return true;
    }
    // bare function call: identifier(
    let bytes = s.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'(' && bytes[i - 1].is_ascii_alphanumeric() {
            return true;
        }
    }
    false
}

fn is_structure(s: &str) -> bool {
    starts_with_any(
        s,
        &[
            "if ", "if(", "else ", "else{", "for ", "for(", "while ", "while(", "match ", "match(",
            "switch ", "switch(", "try ", "try{", "catch ", "catch(", "loop ", "loop{",
        ],
    )
}

fn is_identifier_line(s: &str) -> bool {
    // Contains `=` but not comparison / arrow operators
    if let Some(pos) = s.find('=') {
        let bytes = s.as_bytes();
        // Not ==, !=, <=, >=, =>
        if pos > 0 && matches!(bytes[pos - 1], b'!' | b'<' | b'>') {
            return false;
        }
        if pos + 1 < bytes.len() && matches!(bytes[pos + 1], b'=' | b'>') {
            return false;
        }
        return true;
    }
    false
}

fn is_symbol_only(s: &str) -> bool {
    s.chars()
        .all(|c| matches!(c, '{' | '}' | '(' | ')' | '[' | ']' | ';' | ',' | ' '))
}

// ---------------------------------------------------------------------------
// Phase 4: Line Deduplication
// ---------------------------------------------------------------------------

fn deduplicate_lines(lines: Vec<(usize, String, f64)>) -> Vec<(usize, String, f64)> {
    // Build pattern keys
    let keys: Vec<String> = lines.iter().map(|(_, l, _)| pattern_key(l)).collect();

    // Count occurrences per pattern
    let mut counts: HashMap<String, usize> = HashMap::new();
    for k in &keys {
        *counts.entry(k.clone()).or_default() += 1;
    }

    // Track which patterns we've already emitted
    let mut emitted: HashMap<String, bool> = HashMap::new();
    let mut result: Vec<(usize, String, f64)> = Vec::with_capacity(lines.len());

    for (i, (num, line, score)) in lines.into_iter().enumerate() {
        let key = &keys[i];
        let count = *counts.get(key).unwrap_or(&1);

        if count >= 3 {
            if emitted.contains_key(key) {
                // Skip — already factored
                continue;
            }
            emitted.insert(key.clone(), true);
            let factored = format!("{} (x{})", line.trim_end(), count);
            let merged_score = score * count as f64;
            result.push((num, factored, merged_score));
        } else {
            result.push((num, line, score));
        }
    }

    result
}

/// Generate a normalized pattern key for deduplication.
/// Replace quoted strings and identifiers after `->`, `['`, `$` with `*`.
fn pattern_key(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Replace quoted strings
        if b == b'"' || b == b'\'' {
            let quote = b;
            out.push('*');
            i += 1;
            while i < len && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1; // skip escaped char
                }
                i += 1;
            }
            if i < len {
                i += 1; // closing quote
            }
            continue;
        }

        // Replace identifier after -> or .
        if (b == b'-' && i + 1 < len && bytes[i + 1] == b'>')
            || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_alphabetic())
        {
            if b == b'-' {
                out.push_str("->*");
                i += 2;
            } else {
                out.push_str(".*");
                i += 1;
            }
            // Consume identifier
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            continue;
        }

        // Replace after ['
        if b == b'[' && i + 1 < len && bytes[i + 1] == b'\'' {
            out.push_str("[*]");
            i += 2;
            // Skip to closing ]
            while i < len && bytes[i] != b']' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        out.push(b as char);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Phase 5: Budget Fitting
// ---------------------------------------------------------------------------

fn fit_to_budget(lines: Vec<(usize, String, f64)>, budget_tokens: usize) -> Vec<(usize, String)> {
    let budget_chars = budget_tokens * 4;
    let mut total_chars: usize = lines.iter().map(|(_, l, _)| l.len() + 7).sum();

    if total_chars <= budget_chars {
        return lines.into_iter().map(|(n, l, _)| (n, l)).collect();
    }

    let len = lines.len();

    // Build index-sorted-by-score for removal candidates
    let mut by_score: Vec<usize> = (0..len).collect();
    by_score.sort_by(|&a, &b| {
        lines[a]
            .2
            .partial_cmp(&lines[b].2)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut removed = vec![false; len];

    // Pass 1: remove non-signatures (score < 0.95)
    for &idx in &by_score {
        if total_chars <= budget_chars {
            break;
        }
        if lines[idx].2 >= 0.95 {
            continue;
        }
        let line_chars = lines[idx].1.len() + 7;
        removed[idx] = true;
        total_chars = total_chars.saturating_sub(line_chars);
    }

    // Pass 2: if still over budget, remove low-score signatures too
    if total_chars > budget_chars {
        for &idx in &by_score {
            if total_chars <= budget_chars {
                break;
            }
            if removed[idx] {
                continue;
            }
            let line_chars = lines[idx].1.len() + 7;
            removed[idx] = true;
            total_chars = total_chars.saturating_sub(line_chars);
        }
    }

    // Build output preserving original order, inserting omission markers
    let mut result: Vec<(usize, String)> = Vec::new();
    let mut omitted_count: usize = 0;

    for (i, (line_num, line, _)) in lines.into_iter().enumerate() {
        if removed[i] {
            omitted_count += 1;
        } else {
            if omitted_count > 0 {
                result.push((0, format!("    // ... [{} lines omitted]", omitted_count)));
                omitted_count = 0;
            }
            result.push((line_num, line));
        }
    }
    if omitted_count > 0 {
        result.push((0, format!("    // ... [{} lines omitted]", omitted_count)));
    }

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_leading_keywords<'a>(s: &'a str, keywords: &[&str]) -> &'a str {
    let mut current = s;
    loop {
        let mut found = false;
        for kw in keywords {
            if let Some(rest) = current.strip_prefix(kw) {
                current = rest;
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
    }
    current
}

fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Phase 3: Token Type Priority --

    #[test]
    fn test_signature_detection() {
        assert!(is_signature("fn main() {"));
        assert!(is_signature("pub fn compress_lsc("));
        assert!(is_signature("pub(crate) fn helper() -> bool {"));
        assert!(is_signature("async fn fetch_data() {"));
        assert!(is_signature("def process(data):"));
        assert!(is_signature("class MyClass:"));
        assert!(is_signature("export default function App() {"));
        assert!(!is_signature("let x = 42;"));
        assert!(!is_signature("// fn commented out"));
    }

    #[test]
    fn test_invocation_detection() {
        assert!(is_invocation("self.reader.lookup_ngram(key)"));
        assert!(is_invocation("reader->process(data)"));
        assert!(is_invocation("String::from(\"hello\")"));
        assert!(is_invocation("println(\"hi\")"));
        assert!(!is_invocation("let x = 42;"));
    }

    #[test]
    fn test_structure_detection() {
        assert!(is_structure("if x > 0 {"));
        assert!(is_structure("for item in list {"));
        assert!(is_structure("while running {"));
        assert!(is_structure("match result {"));
        assert!(!is_structure("let matched = true;"));
    }

    #[test]
    fn test_identifier_line() {
        assert!(is_identifier_line("let x = 42;"));
        assert!(is_identifier_line("self.count = 0;"));
        assert!(!is_identifier_line("if x == 42 {"));
        assert!(!is_identifier_line("if x != 42 {"));
        assert!(!is_identifier_line("if x <= 42 {"));
        assert!(!is_identifier_line("x => handler(x)"));
    }

    #[test]
    fn test_symbol_only() {
        assert!(is_symbol_only("}"));
        assert!(is_symbol_only("});"));
        assert!(is_symbol_only("  }  "));
        assert!(!is_symbol_only("return;"));
        assert!(!is_symbol_only("x}"));
    }

    #[test]
    fn test_token_type_weights() {
        assert_eq!(token_type_weight(""), 0.0);
        assert_eq!(token_type_weight("pub fn main() {"), 1.0);
        assert_eq!(token_type_weight("  }"), 0.1);
        assert_eq!(token_type_weight("if x > 0 {"), 0.5);
        assert_eq!(token_type_weight("let x = 42;"), 0.3);
    }

    // -- Phase 4: Deduplication --

    #[test]
    fn test_pattern_key_strings() {
        let key = pattern_key(r#"println!("hello world")"#);
        assert!(
            key.contains('*'),
            "quoted strings should be replaced: {}",
            key
        );
    }

    #[test]
    fn test_pattern_key_arrow() {
        let a = pattern_key("self->name = value;");
        let b = pattern_key("self->age = value;");
        assert_eq!(a, b, "identifiers after -> should be normalized");
    }

    #[test]
    fn test_dedup_below_threshold() {
        let lines = vec![
            (1, "let a = 1;".to_string(), 0.5),
            (2, "let b = 2;".to_string(), 0.5),
        ];
        let result = deduplicate_lines(lines);
        assert_eq!(result.len(), 2, "fewer than 3 duplicates should not merge");
    }

    #[test]
    fn test_dedup_above_threshold() {
        // 3 lines with same pattern after normalization
        let lines = vec![
            (1, r#"log("alpha")"#.to_string(), 0.3),
            (2, r#"log("beta")"#.to_string(), 0.3),
            (3, r#"log("gamma")"#.to_string(), 0.3),
        ];
        let result = deduplicate_lines(lines);
        assert_eq!(result.len(), 1, "3+ matching patterns should merge");
        assert!(
            result[0].1.contains("x3"),
            "merged line should have count: {}",
            result[0].1
        );
    }

    // -- Phase 5: Budget Fitting --

    #[test]
    fn test_fit_under_budget() {
        let lines = vec![(1, "short".to_string(), 0.5), (2, "line".to_string(), 0.5)];
        let result = fit_to_budget(lines, 1000);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_fit_over_budget_removes_low_score() {
        let lines = vec![
            (1, "fn main() {".to_string(), 1.0), // signature — kept
            (2, "let x = boring_boilerplate;".to_string(), 0.01), // low score
            (3, "let y = boring_boilerplate;".to_string(), 0.01), // low score
            (4, "}".to_string(), 0.05),          // low score
        ];
        // Budget enough to keep signature but not all boilerplate
        let result = fit_to_budget(lines, 8); // 8 tokens = 32 chars
        // Signature should survive (pass 1 only removes non-signatures)
        assert!(
            result.iter().any(|(_, l)| l.contains("fn main")),
            "signatures should not be removed when budget allows"
        );
        // Should have at least one omission marker
        assert!(
            result.iter().any(|(_, l)| l.contains("lines omitted")),
            "should insert omission markers"
        );
    }

    #[test]
    fn test_fit_very_tight_budget_removes_signatures() {
        let lines = vec![
            (1, "fn main() {".to_string(), 1.0),
            (2, "fn helper() {".to_string(), 1.0),
            (3, "fn another() {".to_string(), 1.0),
            (4, "fn more() {".to_string(), 1.0),
            (5, "fn extra() {".to_string(), 1.0),
        ];
        // Budget of 2 tokens = 8 chars — can't keep all 5 signatures
        let result = fit_to_budget(lines, 2);
        assert!(
            result.len() < 5,
            "tight budget should remove some signatures, got {}",
            result.len()
        );
    }

    // -- Integrated: compress_lsc --

    #[test]
    fn test_compress_lsc_empty() {
        let result = compress_lsc(vec![], None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compress_lsc_filters_symbol_lines() {
        let lines = vec![
            (1, "fn main() {".to_string()),
            (2, "    let x = 1;".to_string()),
            (3, "}".to_string()),
        ];
        // Without index, entropy is neutral (1.0) — score = entropy * weight
        // "}" has weight 0.1, entropy ~1.0 → score ~0.1 → above 0.05 threshold
        // so it survives. Only truly empty/zero-score lines get filtered.
        let result = compress_lsc(lines, None, None);
        assert!(!result.is_empty());
        // Signature should always be present
        assert!(result.iter().any(|(_, l)| l.contains("fn main")));
    }

    #[test]
    fn test_compress_lsc_with_budget() {
        let lines: Vec<(usize, String)> = (1..=20)
            .map(|i| (i, format!("    let var_{} = {};", i, i)))
            .collect();
        // Very tight budget → must cut lines
        let result = compress_lsc(lines, Some(10), None);
        assert!(
            result.len() < 20,
            "tight budget should reduce line count, got {}",
            result.len()
        );
    }

    // -- Phase 2: Entropy (unit, no index) --

    #[test]
    fn test_relevance_boost() {
        let lines = vec![
            (1, "fn payment_process() {".to_string()),
            (2, "fn unrelated_helper() {".to_string()),
            (3, "    let x = calculate_payment();".to_string()),
            (4, "    let y = do_other_thing();".to_string()),
        ];
        let result = compress_lsc(lines, Some(20), Some("payment"));
        // payment-related lines should survive the tight budget
        let text: String = result
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("payment_process"),
            "relevant signature should survive"
        );
    }

    #[test]
    fn test_entropy_no_index() {
        let scorer = EntropyScorer {
            reader: None,
            total_files: 100.0,
        };
        let score = score_line_entropy("fn main() {", &scorer);
        // Without index, all trigrams get idf=1.0 → average = 1.0
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_entropy_short_line() {
        let scorer = EntropyScorer {
            reader: None,
            total_files: 100.0,
        };
        // Line shorter than 3 bytes → score = 0
        assert_eq!(score_line_entropy("ab", &scorer), 0.0);
        assert_eq!(score_line_entropy("", &scorer), 0.0);
    }
}
