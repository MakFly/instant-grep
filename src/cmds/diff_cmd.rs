//! `ig diff <file1> <file2>` — ultra-condensed file diff.
//!
//! Reads both files, performs a simple line-by-line comparison, and shows
//! only changed lines with +/- prefixes and unchanged-line markers.

use anyhow::Result;
use std::path::Path;

/// Run the diff command.
pub fn run(args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        anyhow::bail!("Usage: ig diff <file1> <file2>");
    }

    let content_a = std::fs::read_to_string(Path::new(&args[0]))?;
    let content_b = std::fs::read_to_string(Path::new(&args[1]))?;

    let lines_a: Vec<&str> = content_a.lines().collect();
    let lines_b: Vec<&str> = content_b.lines().collect();

    let diff = compute_diff(&lines_a, &lines_b);
    print_condensed(&diff);

    Ok(0)
}

#[derive(Debug, PartialEq)]
enum DiffLine<'a> {
    Same(&'a str),
    Added(&'a str),
    Removed(&'a str),
}

/// Simple LCS-based diff between two line slices.
fn compute_diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<DiffLine<'a>> {
    let m = a.len();
    let n = b.len();

    // Build LCS table
    let mut table = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce diff
    let mut result = Vec::new();
    let mut i = m;
    let mut j = n;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            result.push(DiffLine::Same(a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            result.push(DiffLine::Added(b[j - 1]));
            j -= 1;
        } else {
            result.push(DiffLine::Removed(a[i - 1]));
            i -= 1;
        }
    }

    result.reverse();
    result
}

/// Print the diff in condensed form: show changes and collapse unchanged runs.
fn print_condensed(diff: &[DiffLine<'_>]) {
    let mut unchanged_count = 0;

    for line in diff {
        match line {
            DiffLine::Same(_) => {
                unchanged_count += 1;
            }
            DiffLine::Added(text) => {
                flush_unchanged(&mut unchanged_count);
                println!("+ {}", text);
            }
            DiffLine::Removed(text) => {
                flush_unchanged(&mut unchanged_count);
                println!("- {}", text);
            }
        }
    }

    flush_unchanged(&mut unchanged_count);
}

fn flush_unchanged(count: &mut usize) {
    if *count > 0 {
        println!("... {} unchanged lines ...", count);
        *count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files() {
        let a = vec!["hello", "world"];
        let b = vec!["hello", "world"];
        let diff = compute_diff(&a, &b);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| matches!(d, DiffLine::Same(_))));
    }

    #[test]
    fn added_line() {
        let a = vec!["hello"];
        let b = vec!["hello", "world"];
        let diff = compute_diff(&a, &b);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0], DiffLine::Same("hello"));
        assert_eq!(diff[1], DiffLine::Added("world"));
    }

    #[test]
    fn removed_line() {
        let a = vec!["hello", "world"];
        let b = vec!["hello"];
        let diff = compute_diff(&a, &b);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0], DiffLine::Same("hello"));
        assert_eq!(diff[1], DiffLine::Removed("world"));
    }

    #[test]
    fn changed_line() {
        let a = vec!["hello", "world"];
        let b = vec!["hello", "earth"];
        let diff = compute_diff(&a, &b);
        assert!(diff.contains(&DiffLine::Removed("world")));
        assert!(diff.contains(&DiffLine::Added("earth")));
    }
}
