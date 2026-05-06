pub type DocId = u32;

/// Ratio threshold: if one list is >GALLOP_RATIO times larger than the other,
/// use galloping intersection instead of linear merge.
const GALLOP_RATIO: usize = 4;

/// Intersect two sorted slices of DocIds.
///
/// Automatically selects the best algorithm:
/// - **Linear merge** O(m + n) when both lists are similarly sized.
/// - **Galloping intersection** O(m log n) when one list is much shorter,
///   which avoids scanning the long list linearly.
pub fn intersect(a: &[DocId], b: &[DocId]) -> Vec<DocId> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }

    // Pick galloping when one side is significantly smaller.
    if a.len() * GALLOP_RATIO < b.len() {
        return intersect_galloping(a, b);
    }
    if b.len() * GALLOP_RATIO < a.len() {
        return intersect_galloping(b, a);
    }

    intersect_merge(a, b)
}

/// Branchless two-pointer merge intersection — O(m + n).
/// Uses conditional moves instead of branches to avoid misprediction penalties.
fn intersect_merge(a: &[DocId], b: &[DocId]) -> Vec<DocId> {
    let mut result = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        let va = a[i];
        let vb = b[j];
        // Branchless: always compute both increments, select via comparison
        let eq = (va == vb) as usize;
        let lt = (va < vb) as usize;
        if eq != 0 {
            result.push(va);
        }
        // Advance i when va <= vb, advance j when va >= vb
        i += eq | lt;
        j += eq | (1 - lt);
    }

    result
}

/// Galloping (exponential-search) intersection — O(m log n) where m = short.len().
///
/// For each element in `short`, performs an exponential search in `long` to find
/// the matching position. Because `long` is scanned forward-only, earlier matches
/// narrow the search window for subsequent elements.
pub fn intersect_galloping(short: &[DocId], long: &[DocId]) -> Vec<DocId> {
    let mut result = Vec::with_capacity(short.len());
    let mut lo = 0;

    for &val in short {
        lo = gallop(long, val, lo);
        if lo < long.len() && long[lo] == val {
            result.push(val);
            lo += 1;
        }
    }

    result
}

/// Galloping search: find the first index `>= start` in `data` where `data[idx] >= target`.
///
/// 1. Exponential probe: double the step size until we overshoot `target`.
/// 2. Binary search within the narrowed range.
///
/// Returns the index of `target` if present, or the insertion point otherwise.
fn gallop(data: &[DocId], target: DocId, start: usize) -> usize {
    if start >= data.len() {
        return data.len();
    }

    // Fast exit: current position already >= target
    if data[start] >= target {
        return start;
    }

    // Exponential search: find an upper bound
    let mut bound = 1usize;
    while start + bound < data.len() && data[start + bound] < target {
        bound *= 2;
    }

    // Binary search within [start + bound/2, min(start + bound, len))
    let lo = start + bound / 2;
    let hi = (start + bound).min(data.len());
    lo + data[lo..hi].partition_point(|&x| x < target)
}

/// Union two sorted slices of DocIds.
pub fn union(a: &[DocId], b: &[DocId]) -> Vec<DocId> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                result.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                result.push(b[j]);
                j += 1;
            }
        }
    }

    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}

// ---------------------------------------------------------------------------
// Streaming intersection (zero intermediate allocations)
// ---------------------------------------------------------------------------

use crate::index::vbyte::PostingIterator;

/// Two-way streaming intersection using PostingIterators.
/// Uses advance_to() for efficient skipping — no posting list is fully decoded.
#[allow(dead_code)]
pub fn intersect_two_iters(a: &mut PostingIterator, b: &mut PostingIterator) -> Vec<DocId> {
    let mut result = Vec::with_capacity(a.doc_count().min(b.doc_count()) as usize);

    while let (Some(va), Some(vb)) = (a.current(), b.current()) {
        if va == vb {
            result.push(va);
            a.advance();
            b.advance();
        } else if va < vb {
            if a.advance_to(vb).is_none() {
                break;
            }
        } else {
            if b.advance_to(va).is_none() {
                break;
            }
        }
    }

    result
}

/// Intersect a materialized Vec with a PostingIterator (for chained multi-way intersection).
#[allow(dead_code)]
pub fn intersect_vec_iter(sorted: &[DocId], iter: &mut PostingIterator) -> Vec<DocId> {
    let mut result = Vec::with_capacity(sorted.len().min(iter.doc_count() as usize));

    for &val in sorted {
        match iter.advance_to(val) {
            Some(v) if v == val => {
                result.push(val);
                iter.advance();
            }
            Some(_) => {} // overshot, continue with next sorted value
            None => break,
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intersect() {
        assert_eq!(intersect(&[1, 3, 5, 7], &[2, 3, 5, 8]), vec![3, 5]);
        assert_eq!(intersect(&[1, 2, 3], &[4, 5, 6]), Vec::<DocId>::new());
        assert_eq!(intersect(&[1, 2, 3], &[1, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn test_intersect_empty() {
        assert_eq!(intersect(&[], &[1, 2, 3]), Vec::<DocId>::new());
        assert_eq!(intersect(&[1, 2, 3], &[]), Vec::<DocId>::new());
        assert_eq!(intersect(&[], &[]), Vec::<DocId>::new());
    }

    #[test]
    fn test_intersect_galloping_direct() {
        // Directly test galloping path with asymmetric lists
        let short = vec![5, 50, 500];
        let long: Vec<DocId> = (0..1000).collect();
        assert_eq!(intersect_galloping(&short, &long), vec![5, 50, 500]);
    }

    #[test]
    fn test_intersect_galloping_no_match() {
        let short = vec![1001, 2000, 3000];
        let long: Vec<DocId> = (0..1000).collect();
        assert_eq!(intersect_galloping(&short, &long), Vec::<DocId>::new());
    }

    #[test]
    fn test_intersect_auto_selects_galloping() {
        // One list is >4x larger → should automatically use galloping
        let short = vec![10, 100, 500, 999];
        let long: Vec<DocId> = (0..1000).collect();
        assert_eq!(intersect(&short, &long), vec![10, 100, 500, 999]);
        // Reversed argument order should also work
        assert_eq!(intersect(&long, &short), vec![10, 100, 500, 999]);
    }

    #[test]
    fn test_intersect_galloping_boundaries() {
        // First and last elements
        let short = vec![0, 999];
        let long: Vec<DocId> = (0..1000).collect();
        assert_eq!(intersect_galloping(&short, &long), vec![0, 999]);
    }

    #[test]
    fn test_intersect_galloping_consecutive() {
        // All elements consecutive — exercises the lo advancement
        let short = vec![5, 6, 7, 8, 9];
        let long: Vec<DocId> = (0..100).collect();
        assert_eq!(intersect_galloping(&short, &long), vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_intersect_galloping_single() {
        assert_eq!(intersect_galloping(&[42], &[10, 20, 30, 42, 50]), vec![42]);
        assert_eq!(
            intersect_galloping(&[99], &[10, 20, 30, 42, 50]),
            Vec::<DocId>::new()
        );
    }

    #[test]
    fn test_union() {
        assert_eq!(union(&[1, 3, 5], &[2, 4, 6]), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(union(&[1, 2, 3], &[2, 3, 4]), vec![1, 2, 3, 4]);
    }

    // -- Streaming intersection tests --

    use crate::index::vbyte::{PostingIterator, encode_posting_list};

    #[test]
    fn test_intersect_two_iters() {
        let a_ids: Vec<DocId> = vec![1, 3, 5, 7, 9];
        let b_ids: Vec<DocId> = vec![2, 3, 5, 8, 9];
        let a_enc = encode_posting_list(&a_ids);
        let b_enc = encode_posting_list(&b_ids);
        let mut a = PostingIterator::new(&a_enc, 0, a_enc.len());
        let mut b = PostingIterator::new(&b_enc, 0, b_enc.len());
        assert_eq!(intersect_two_iters(&mut a, &mut b), vec![3, 5, 9]);
    }

    #[test]
    fn test_intersect_two_iters_no_overlap() {
        let a_enc = encode_posting_list(&[1, 3, 5]);
        let b_enc = encode_posting_list(&[2, 4, 6]);
        let mut a = PostingIterator::new(&a_enc, 0, a_enc.len());
        let mut b = PostingIterator::new(&b_enc, 0, b_enc.len());
        assert_eq!(intersect_two_iters(&mut a, &mut b), Vec::<DocId>::new());
    }

    #[test]
    fn test_intersect_two_iters_identical() {
        let ids: Vec<DocId> = vec![10, 20, 30];
        let enc = encode_posting_list(&ids);
        let enc2 = encode_posting_list(&ids);
        let mut a = PostingIterator::new(&enc, 0, enc.len());
        let mut b = PostingIterator::new(&enc2, 0, enc2.len());
        assert_eq!(intersect_two_iters(&mut a, &mut b), vec![10, 20, 30]);
    }

    #[test]
    fn test_intersect_two_iters_skewed() {
        let short: Vec<DocId> = vec![50, 500];
        let long: Vec<DocId> = (0..1000).collect();
        let s_enc = encode_posting_list(&short);
        let l_enc = encode_posting_list(&long);
        let mut s = PostingIterator::new(&s_enc, 0, s_enc.len());
        let mut l = PostingIterator::new(&l_enc, 0, l_enc.len());
        assert_eq!(intersect_two_iters(&mut s, &mut l), vec![50, 500]);
    }

    #[test]
    fn test_intersect_vec_iter() {
        let sorted: Vec<DocId> = vec![3, 5, 9, 15];
        let b_ids: Vec<DocId> = vec![1, 3, 7, 9, 12, 15, 20];
        let b_enc = encode_posting_list(&b_ids);
        let mut b = PostingIterator::new(&b_enc, 0, b_enc.len());
        assert_eq!(intersect_vec_iter(&sorted, &mut b), vec![3, 9, 15]);
    }

    #[test]
    fn test_intersect_vec_iter_empty() {
        let sorted: Vec<DocId> = vec![1, 2, 3];
        let b_enc = encode_posting_list(&[10, 20]);
        let mut b = PostingIterator::new(&b_enc, 0, b_enc.len());
        assert_eq!(intersect_vec_iter(&sorted, &mut b), Vec::<DocId>::new());
    }

    #[test]
    fn test_streaming_matches_materialized() {
        // Verify streaming intersection gives same results as materialized
        let a_ids: Vec<DocId> = vec![1, 5, 10, 15, 20, 25, 30, 50, 100];
        let b_ids: Vec<DocId> = vec![3, 5, 8, 15, 22, 25, 30, 45, 100, 200];
        let expected = intersect(&a_ids, &b_ids);

        let a_enc = encode_posting_list(&a_ids);
        let b_enc = encode_posting_list(&b_ids);
        let mut a = PostingIterator::new(&a_enc, 0, a_enc.len());
        let mut b = PostingIterator::new(&b_enc, 0, b_enc.len());
        let streaming_result = intersect_two_iters(&mut a, &mut b);

        assert_eq!(streaming_result, expected);
    }
}
