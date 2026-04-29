use anyhow::{Context, Result};
use regex_syntax::Parser;
use regex_syntax::hir::literal::{ExtractKind, Extractor, Literal, Seq};

use crate::index::ngram::{
    BigramDfTable, DEFAULT_MAX_NGRAM_LEN, bloom_bit, build_covering_ngrams, hash_ngram,
};
use crate::query::plan::NgramQuery;

/// Convert a regex pattern into an NgramQuery using sparse n-grams.
///
/// Uses regex-syntax's Extractor to pull prefix/suffix literals from the pattern,
/// then applies the covering algorithm to extract sparse n-gram keys.
#[allow(dead_code)]
pub fn regex_to_query(
    pattern: &str,
    case_insensitive: bool,
    df_table: Option<&BigramDfTable>,
) -> Result<NgramQuery> {
    regex_to_query_with_picker(pattern, case_insensitive, df_table, |prefix, suffix| {
        choose_by_literal_quality(prefix, suffix)
    })
}

pub fn regex_to_query_costed<F>(
    pattern: &str,
    case_insensitive: bool,
    df_table: Option<&BigramDfTable>,
    estimate_cost: F,
) -> Result<NgramQuery>
where
    F: Fn(&NgramQuery) -> u64,
{
    regex_to_query_with_picker(pattern, case_insensitive, df_table, |prefix, suffix| {
        choose_by_estimated_cost(prefix, suffix, &estimate_cost)
    })
}

fn regex_to_query_with_picker<F>(
    pattern: &str,
    case_insensitive: bool,
    df_table: Option<&BigramDfTable>,
    pick: F,
) -> Result<NgramQuery>
where
    F: Fn(Option<QueryCandidate>, Option<QueryCandidate>) -> Option<NgramQuery>,
{
    let hir = Parser::new()
        .parse(pattern)
        .context("invalid regex pattern")?;
    let anchored_start = has_absolute_start_anchor(pattern);

    let mut extractor = Extractor::new();
    extractor.limit_total(256);
    extractor.limit_class(10);

    extractor.kind(ExtractKind::Prefix);
    let prefix_seq = extractor.extract(&hir);

    extractor.kind(ExtractKind::Suffix);
    let suffix_seq = extractor.extract(&hir);

    let prefix = query_candidate(prefix_seq, case_insensitive, df_table, anchored_start);
    let suffix = query_candidate(suffix_seq, case_insensitive, df_table, false);

    if anchored_start && let Some(prefix) = prefix.clone() {
        return Ok(prefix.query);
    }

    Ok(pick(prefix, suffix).unwrap_or(NgramQuery::All))
}

#[derive(Clone)]
struct QueryCandidate {
    query: NgramQuery,
    quality: usize,
}

fn query_candidate(
    seq: Seq,
    case_insensitive: bool,
    df_table: Option<&BigramDfTable>,
    exact_pos: bool,
) -> Option<QueryCandidate> {
    if !seq.is_finite() {
        return None;
    }
    let literals = seq.literals()?;
    if literals.is_empty() {
        return None;
    }
    let quality = literal_quality(literals);
    let query = query_from_literals(literals, case_insensitive, df_table, exact_pos)?;
    Some(QueryCandidate { query, quality })
}

fn query_from_literals(
    literals: &[Literal],
    case_insensitive: bool,
    df_table: Option<&BigramDfTable>,
    exact_pos: bool,
) -> Option<NgramQuery> {
    let mut or_branches = Vec::new();

    for lit in literals {
        let bytes = lit.as_bytes();

        let working_bytes: Vec<u8> = if case_insensitive {
            bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
        } else {
            bytes.to_vec()
        };

        if working_bytes.len() < 2 {
            return None;
        }

        // For 2-byte literals, use bigram key directly (indexed since v0.4)
        if working_bytes.len() == 2 {
            let key = hash_ngram(&working_bytes);
            or_branches.push(NgramQuery::MaskedNgram {
                key,
                next_mask: 0,
                rel_pos: 0,
                exact_pos,
            });
            continue;
        }

        // Use covering algorithm for sparse n-grams
        let ranges = build_covering_ngrams(&working_bytes, DEFAULT_MAX_NGRAM_LEN, df_table);

        if ranges.is_empty() {
            return None;
        }

        let and_query = NgramQuery::And(
            ranges
                .into_iter()
                .map(|(start, end)| NgramQuery::MaskedNgram {
                    key: hash_ngram(&working_bytes[start..end]),
                    next_mask: if end < working_bytes.len() {
                        bloom_bit(working_bytes[end])
                    } else {
                        0
                    },
                    rel_pos: start as u16,
                    exact_pos,
                })
                .collect(),
        );
        or_branches.push(and_query);
    }

    if or_branches.len() == 1 {
        Some(or_branches.pop().unwrap())
    } else {
        Some(NgramQuery::Or(or_branches))
    }
}

fn has_absolute_start_anchor(pattern: &str) -> bool {
    let p = pattern.strip_prefix("(?-m)").unwrap_or(pattern);
    if p.starts_with("\\A") {
        return true;
    }
    if p.starts_with('^') {
        return !pattern.starts_with("(?m)") && !pattern.starts_with("(?m:");
    }
    false
}

#[allow(dead_code)]
fn choose_by_literal_quality(
    prefix: Option<QueryCandidate>,
    suffix: Option<QueryCandidate>,
) -> Option<NgramQuery> {
    match (prefix, suffix) {
        (Some(p), Some(s)) => {
            if s.quality > p.quality {
                Some(s.query)
            } else {
                Some(p.query)
            }
        }
        (Some(p), None) => Some(p.query),
        (None, Some(s)) => Some(s.query),
        (None, None) => None,
    }
}

fn choose_by_estimated_cost<F>(
    prefix: Option<QueryCandidate>,
    suffix: Option<QueryCandidate>,
    estimate_cost: &F,
) -> Option<NgramQuery>
where
    F: Fn(&NgramQuery) -> u64,
{
    match (prefix, suffix) {
        (Some(p), Some(s)) => {
            let p_cost = estimate_cost(&p.query);
            let s_cost = estimate_cost(&s.query);
            let suffix_is_decisively_cheaper = s_cost.saturating_mul(4) < p_cost.saturating_mul(3);
            let prefix_is_decisively_cheaper = p_cost.saturating_mul(4) < s_cost.saturating_mul(3);
            if suffix_is_decisively_cheaper
                || (!prefix_is_decisively_cheaper && s.quality > p.quality)
            {
                Some(s.query)
            } else {
                Some(p.query)
            }
        }
        (Some(p), None) => Some(p.query),
        (None, Some(s)) => Some(s.query),
        (None, None) => None,
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
        let query = regex_to_query("foo_bar", false, None).unwrap();
        match query {
            NgramQuery::And(children) => {
                assert!(!children.is_empty());
            }
            _ => panic!("expected And query, got {:?}", query),
        }
    }

    #[test]
    fn test_alternation_pattern() {
        let query = regex_to_query("foo|bar", false, None).unwrap();
        match query {
            NgramQuery::Or(branches) => {
                assert_eq!(branches.len(), 2);
            }
            _ => panic!("expected Or query, got {:?}", query),
        }
    }

    #[test]
    fn test_wildcard_is_all() {
        let query = regex_to_query(".*", false, None).unwrap();
        assert!(query.is_all());
    }

    #[test]
    fn test_bigram_uses_index() {
        let query = regex_to_query("ab", false, None).unwrap();
        // 2-char patterns should use bigram index lookup, not brute-force
        match query {
            NgramQuery::MaskedNgram { .. } => {}
            _ => panic!("expected Ngram query for bigram, got {:?}", query),
        }
    }

    #[test]
    fn test_single_char_is_all() {
        let query = regex_to_query("a", false, None).unwrap();
        assert!(query.is_all());
    }

    #[test]
    fn test_case_insensitive_extracts_ngrams() {
        let query = regex_to_query("FooBar", true, None).unwrap();
        assert!(!query.is_all());
    }

    #[test]
    fn test_covering_produces_fewer_keys() {
        // A long literal should produce fewer covering n-grams than trigrams
        let query = regex_to_query("fetchSellerListingsAction", false, None).unwrap();
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
