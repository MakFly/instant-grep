use anyhow::{Context, Result};
use regex_syntax::hir::literal::{ExtractKind, Extractor};
use regex_syntax::Parser;

use crate::index::trigram::{pack_trigram, Trigram};
use crate::query::plan::TrigramQuery;

/// Convert a regex pattern into a TrigramQuery.
///
/// Uses regex-syntax's literal Extractor to pull prefix literals from the pattern,
/// then converts those literals into trigram AND/OR queries.
///
/// Returns TrigramQuery::All if the pattern cannot be optimized with trigrams.
pub fn regex_to_query(pattern: &str, case_insensitive: bool) -> Result<TrigramQuery> {
    // For case-insensitive, parse without (?i) to get clean literals,
    // then lowercase them for trigram lookup.
    let hir = Parser::new()
        .parse(pattern)
        .context("invalid regex pattern")?;

    let mut extractor = Extractor::new();
    extractor.limit_total(256);
    extractor.limit_class(10);

    // Try prefix extraction first
    extractor.kind(ExtractKind::Prefix);
    let prefix_seq = extractor.extract(&hir);

    // Also try suffix extraction
    extractor.kind(ExtractKind::Suffix);
    let suffix_seq = extractor.extract(&hir);

    // Pick the more specific one (fewest literals, or longest literals)
    let seq = match (prefix_seq.literals(), suffix_seq.literals()) {
        (Some(p), Some(s)) => {
            let p_quality = literal_quality(p);
            let s_quality = literal_quality(s);
            if s_quality > p_quality {
                suffix_seq
            } else {
                prefix_seq
            }
        }
        (Some(_), None) => prefix_seq,
        (None, Some(_)) => suffix_seq,
        (None, None) => return Ok(TrigramQuery::All),
    };

    if !seq.is_finite() {
        return Ok(TrigramQuery::All);
    }

    let literals = match seq.literals() {
        Some(lits) if !lits.is_empty() => lits,
        _ => return Ok(TrigramQuery::All),
    };

    let mut or_branches = Vec::new();

    for lit in literals {
        let bytes = lit.as_bytes();

        // For case-insensitive: lowercase the literal bytes before trigram extraction.
        // This way we can still extract trigrams even with -i flag.
        let working_bytes: Vec<u8> = if case_insensitive {
            bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
        } else {
            bytes.to_vec()
        };

        if working_bytes.len() < 3 {
            return Ok(TrigramQuery::All);
        }

        let trigrams: Vec<Trigram> = working_bytes
            .windows(3)
            .map(|w| pack_trigram(w[0], w[1], w[2]))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let and_query = TrigramQuery::And(
            trigrams.into_iter().map(TrigramQuery::Trigram).collect(),
        );
        or_branches.push(and_query);
    }

    if or_branches.len() == 1 {
        Ok(or_branches.pop().unwrap())
    } else {
        Ok(TrigramQuery::Or(or_branches))
    }
}

/// Score the quality of extracted literals. Higher = better filtering.
fn literal_quality(literals: &[regex_syntax::hir::literal::Literal]) -> usize {
    if literals.is_empty() {
        return 0;
    }
    let total_bytes: usize = literals.iter().map(|l| l.as_bytes().len()).sum();
    let avg_len = total_bytes / literals.len();
    avg_len * 100 / (literals.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_pattern() {
        let query = regex_to_query("foo_bar", false).unwrap();
        match query {
            TrigramQuery::And(children) => {
                assert!(children.len() >= 5);
            }
            _ => panic!("expected And query, got {:?}", query),
        }
    }

    #[test]
    fn test_alternation_pattern() {
        let query = regex_to_query("foo|bar", false).unwrap();
        match query {
            TrigramQuery::Or(branches) => {
                assert_eq!(branches.len(), 2);
            }
            _ => panic!("expected Or query, got {:?}", query),
        }
    }

    #[test]
    fn test_wildcard_is_all() {
        let query = regex_to_query(".*", false).unwrap();
        assert!(query.is_all());
    }

    #[test]
    fn test_short_literal_is_all() {
        let query = regex_to_query("ab", false).unwrap();
        assert!(query.is_all());
    }

    #[test]
    fn test_case_insensitive_extracts_trigrams() {
        // With case_insensitive=true, we should still get trigrams (not All)
        let query = regex_to_query("FooBar", true).unwrap();
        assert!(!query.is_all(), "case-insensitive should still produce trigrams");
    }
}
