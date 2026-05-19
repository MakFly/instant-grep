//! Token-compressed rendering for parsed tool output.

use super::types::{FormatMode, TestFailure, TestResult};

/// Token-compressed formatter for parser outputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenFormatter;

impl TokenFormatter {
    pub fn new() -> Self {
        Self
    }

    /// Render a `TestResult` according to `mode`.
    pub fn format_test_result(&self, r: &TestResult, mode: FormatMode) -> String {
        match mode {
            FormatMode::Compact => format_compact(r),
            FormatMode::Ultra => format_ultra(r),
        }
    }
}

fn format_compact(r: &TestResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Tests: {} passed, {} failed, {} skipped ({} ms)\n",
        r.passed, r.failed, r.skipped, r.duration_ms
    ));
    if r.failures.is_empty() {
        return out;
    }
    out.push_str("\nFailures:\n");
    for (i, f) in r.failures.iter().enumerate() {
        out.push_str(&format!("  {}. {}", i + 1, f.name));
        if let Some(ref file) = f.file {
            if let Some(line) = f.line {
                out.push_str(&format!(" ({}:{})", file, line));
            } else {
                out.push_str(&format!(" ({})", file));
            }
        }
        out.push('\n');
        if !f.message.is_empty() {
            out.push_str(&format!(
                "     {}\n",
                f.message.lines().next().unwrap_or("")
            ));
        }
        if let Some(ref snip) = f.snippet {
            for line in snip.lines().take(5) {
                out.push_str(&format!("       {}\n", line));
            }
        }
    }
    out
}

fn format_ultra(r: &TestResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}P {}F {}S {}ms\n",
        r.passed, r.failed, r.skipped, r.duration_ms
    ));
    for f in &r.failures {
        out.push_str(&format_failure_ultra(f));
        out.push('\n');
    }
    out
}

fn format_failure_ultra(f: &TestFailure) -> String {
    const MAX: usize = 200;
    let loc = match (&f.file, f.line) {
        (Some(file), Some(line)) => format!("{}:{}", file, line),
        (Some(file), None) => file.clone(),
        _ => String::new(),
    };
    let msg = f.message.lines().next().unwrap_or("").trim();
    let head = if loc.is_empty() {
        format!("FAIL {} | {}", f.name, msg)
    } else {
        format!("FAIL {} @ {} | {}", f.name, loc, msg)
    };
    if head.len() > MAX {
        // Char-boundary safe truncate.
        let mut end = MAX;
        while !head.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &head[..end])
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TestResult {
        TestResult {
            passed: 10,
            failed: 2,
            skipped: 1,
            duration_ms: 420,
            failures: vec![
                TestFailure {
                    name: "adds correctly".into(),
                    file: Some("tests/math.rs".into()),
                    line: Some(42),
                    message: "assertion failed: left == right\n  left: 2\n  right: 3".into(),
                    snippet: Some("let r = add(1, 1);\nassert_eq!(r, 3);".into()),
                },
                TestFailure {
                    name: "timeout".into(),
                    file: None,
                    line: None,
                    message: "exceeded 5s".into(),
                    snippet: None,
                },
            ],
        }
    }

    #[test]
    fn ultra_is_shorter_than_compact() {
        let f = TokenFormatter::new();
        let r = sample();
        let c = f.format_test_result(&r, FormatMode::Compact);
        let u = f.format_test_result(&r, FormatMode::Ultra);
        assert!(u.len() < c.len(), "ultra={} compact={}", u.len(), c.len());
    }

    #[test]
    fn ultra_has_no_snippets() {
        let f = TokenFormatter::new();
        let u = f.format_test_result(&sample(), FormatMode::Ultra);
        assert!(!u.contains("let r ="));
    }
}
