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

/// Classic two-pointer merge intersection — O(m + n).
fn intersect_merge(a: &[DocId], b: &[DocId]) -> Vec<DocId> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }

    result
}

/// Galloping (exponential-search) intersection — O(m log n) where m = short.len().
///
/// For each element in `short`, performs an exponential search in `long` to find
/// the matching position. Because `long` is scanned forward-only, earlier matches
/// narrow the search window for subsequent elements.
pub fn intersect_galloping(short: &[DocId], long: &[DocId]) -> Vec<DocId> {
    let mut result = Vec::new();
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
}
