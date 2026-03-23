use anyhow::{Context, Result};
use regex_syntax::Parser;
use regex_syntax::hir::literal::{ExtractKind, Extractor};

use crate::index::ngram::{DEFAULT_MAX_NGRAM_LEN, extract_covering_ngrams};
use crate::query::plan::NgramQuery;

/// Convert a regex pattern into an NgramQuery using sparse n-grams.
///
/// Uses regex-syntax's Extractor to pull prefix/suffix literals from the pattern,
/// then applies the covering algorithm to extract sparse n-gram keys.
pub fn regex_to_query(pattern: &str, case_insensitive: bool) -> Result<NgramQuery> {
    let hir = Parser::new()
        .parse(pattern)
        .context("invalid regex pattern")?;

    let mut extractor = Extractor::new();
    extractor.limit_total(256);
    extractor.limit_class(10);

    extractor.kind(ExtractKind::Prefix);
    let prefix_seq = extractor.extract(&hir);

    extractor.kind(ExtractKind::Suffix);
    let suffix_seq = extractor.extract(&hir);

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
        (None, None) => return Ok(NgramQuery::All),
    };

    if !seq.is_finite() {
        return Ok(NgramQuery::All);
    }

    let literals = match seq.literals() {
        Some(lits) if !lits.is_empty() => lits,
        _ => return Ok(NgramQuery::All),
    };

    let mut or_branches = Vec::new();

    for lit in literals {
        let bytes = lit.as_bytes();

        let working_bytes: Vec<u8> = if case_insensitive {
            bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
        } else {
            bytes.to_vec()
        };

        if working_bytes.len() < 3 {
            return Ok(NgramQuery::All);
        }

        // Use covering algorithm for sparse n-grams
        let ngram_keys = extract_covering_ngrams(&working_bytes, DEFAULT_MAX_NGRAM_LEN);

        if ngram_keys.is_empty() {
            return Ok(NgramQuery::All);
        }

        let and_query = NgramQuery::And(ngram_keys.into_iter().map(NgramQuery::Ngram).collect());
        or_branches.push(and_query);
    }

    if or_branches.len() == 1 {
        Ok(or_branches.pop().unwrap())
    } else {
        Ok(NgramQuery::Or(or_branches))
    }
}

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
            NgramQuery::And(children) => {
                assert!(!children.is_empty());
            }
            _ => panic!("expected And query, got {:?}", query),
        }
    }

    #[test]
    fn test_alternation_pattern() {
        let query = regex_to_query("foo|bar", false).unwrap();
        match query {
            NgramQuery::Or(branches) => {
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
    fn test_case_insensitive_extracts_ngrams() {
        let query = regex_to_query("FooBar", true).unwrap();
        assert!(!query.is_all());
    }

    #[test]
    fn test_covering_produces_fewer_keys() {
        // A long literal should produce fewer covering n-grams than trigrams
        let query = regex_to_query("fetchSellerListingsAction", false).unwrap();
        match query {
            NgramQuery::And(children) => {
                // Covering should produce ~3-6 n-grams instead of ~23 trigrams
                assert!(
                    children.len() < 15,
                    "covering should produce fewer keys, got {}",
                    children.len()
                );
            }
            _ => panic!("expected And query"),
        }
    }
}
