use regex::Regex;

/// A compiled filter ready to match commands and transform output.
#[derive(Debug)]
pub struct CompiledFilter {
    #[allow(dead_code)]
    pub name: String,
    pub match_regex: Regex,
    pub strip_ansi: bool,
    pub replace_rules: Vec<(Regex, String)>,
    pub keep_lines: Option<Regex>,
    pub drop_lines: Option<Regex>,
    pub truncate_at: Option<usize>,
    pub head: Option<usize>,
    pub tail: Option<usize>,
    pub max_lines: Option<usize>,
    pub on_empty: Option<String>,
}

impl CompiledFilter {
    /// Check if this filter matches a command string.
    pub fn matches(&self, command: &str) -> bool {
        self.match_regex.is_match(command)
    }
}

/// Apply the 8-stage pipeline to raw output.
pub fn apply_filter(filter: &CompiledFilter, raw: &str) -> String {
    let mut output = raw.to_string();

    // Stage 1: strip ANSI escape codes
    if filter.strip_ansi {
        output = strip_ansi_codes(&output);
    }

    // Stage 2: regex replacements (line-by-line)
    if !filter.replace_rules.is_empty() {
        output = apply_replacements(&output, &filter.replace_rules);
    }

    // Stage 3: keep/drop lines (mutually exclusive)
    if let Some(ref re) = filter.keep_lines {
        output = keep_matching_lines(&output, re);
    } else if let Some(ref re) = filter.drop_lines {
        output = drop_matching_lines(&output, re);
    }

    // Stage 4: truncate each line to max chars
    if let Some(max_chars) = filter.truncate_at {
        output = truncate_lines(&output, max_chars);
    }

    // Stage 5: keep first N lines
    if let Some(n) = filter.head {
        output = take_head(&output, n);
    }

    // Stage 6: keep last N lines
    if let Some(n) = filter.tail {
        output = take_tail(&output, n);
    }

    // Stage 7: cap total line count
    if let Some(n) = filter.max_lines {
        output = cap_lines(&output, n);
    }

    // Stage 8: fallback message when output is empty
    if let Some(ref msg) = filter.on_empty
        && output.trim().is_empty() {
            output = msg.clone();
        }

    // Cleanup: collapse consecutive blank lines
    output = collapse_blank_lines(&output);

    output
}

/// Remove ANSI escape sequences (colors, cursor movement, etc.).
fn strip_ansi_codes(s: &str) -> String {
    // Matches CSI sequences: ESC [ ... final_byte
    let re = Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
    re.replace_all(s, "").to_string()
}

/// Apply regex replacements line by line. Empty lines after replacement are removed.
fn apply_replacements(s: &str, rules: &[(Regex, String)]) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let mut result = Vec::with_capacity(lines.len());

    for line in lines {
        let mut current = line.to_string();
        for (find, with) in rules {
            current = find.replace_all(&current, with.as_str()).to_string();
        }
        // Only keep lines that aren't empty after replacement
        if !current.is_empty() {
            result.push(current);
        }
    }

    result.join("\n")
}

/// Keep only lines matching the regex.
fn keep_matching_lines(s: &str, re: &Regex) -> String {
    s.lines()
        .filter(|line| re.is_match(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drop lines matching the regex, keep everything else.
fn drop_matching_lines(s: &str, re: &Regex) -> String {
    s.lines()
        .filter(|line| !re.is_match(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate each line to at most `max_chars` characters.
fn truncate_lines(s: &str, max_chars: usize) -> String {
    s.lines()
        .map(|line| {
            if line.chars().count() > max_chars {
                let truncated: String = line.chars().take(max_chars).collect();
                format!("{}…", truncated)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keep only the first `n` lines.
fn take_head(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let total = lines.len();
    if total <= n {
        return s.to_string();
    }
    let kept: Vec<&str> = lines[..n].to_vec();
    let skipped = total - n;
    format!("{}\n… ({} lines omitted)", kept.join("\n"), skipped)
}

/// Keep only the last `n` lines.
fn take_tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let total = lines.len();
    if total <= n {
        return s.to_string();
    }
    let skipped = total - n;
    let kept: Vec<&str> = lines[skipped..].to_vec();
    format!("… ({} lines omitted)\n{}", skipped, kept.join("\n"))
}

/// Cap total output to `n` lines, keeping head and tail with a gap marker.
fn cap_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let total = lines.len();
    if total <= n {
        return s.to_string();
    }
    // Keep 60% head, 40% tail
    let head_count = (n * 3) / 5;
    let tail_count = n - head_count;
    let skipped = total - head_count - tail_count;

    let head_part = &lines[..head_count];
    let tail_part = &lines[total - tail_count..];

    format!(
        "{}\n… ({} lines omitted)\n{}",
        head_part.join("\n"),
        skipped,
        tail_part.join("\n")
    )
}

/// Collapse multiple consecutive blank lines into a single blank line.
fn collapse_blank_lines(s: &str) -> String {
    let mut result = Vec::new();
    let mut prev_blank = false;

    for line in s.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !prev_blank {
                result.push(line);
            }
            prev_blank = true;
        } else {
            prev_blank = false;
            result.push(line);
        }
    }

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_codes() {
        let input = "\x1b[31mERROR\x1b[0m: something failed";
        let result = strip_ansi_codes(input);
        assert_eq!(result, "ERROR: something failed");
    }

    #[test]
    fn test_strip_ansi_codes_no_ansi() {
        let input = "plain text";
        assert_eq!(strip_ansi_codes(input), "plain text");
    }

    #[test]
    fn test_apply_replacements() {
        let rules = vec![
            (Regex::new(r"^\s+Compiling .+$").unwrap(), String::new()),
            (Regex::new(r"warning:").unwrap(), "WARN:".to_string()),
        ];
        let input = "   Compiling foo v1.0\nwarning: unused var\ntest ok";
        let result = apply_replacements(input, &rules);
        assert_eq!(result, "WARN: unused var\ntest ok");
    }

    #[test]
    fn test_keep_matching_lines() {
        let re = Regex::new(r"^(test |error)").unwrap();
        let input = "compiling...\ntest foo ... ok\nerror: failed\nwarning: unused";
        let result = keep_matching_lines(input, &re);
        assert_eq!(result, "test foo ... ok\nerror: failed");
    }

    #[test]
    fn test_drop_matching_lines() {
        let re = Regex::new(r"^warning:").unwrap();
        let input = "error: bad\nwarning: unused\nok";
        let result = drop_matching_lines(input, &re);
        assert_eq!(result, "error: bad\nok");
    }

    #[test]
    fn test_truncate_lines() {
        let input = "short\nthis is a very long line that should be truncated";
        let result = truncate_lines(input, 20);
        assert!(result.contains("short"));
        assert!(result.contains("this is a very long …"));
    }

    #[test]
    fn test_take_head() {
        let input = "a\nb\nc\nd\ne";
        let result = take_head(input, 2);
        assert!(result.starts_with("a\nb"));
        assert!(result.contains("3 lines omitted"));
    }

    #[test]
    fn test_take_head_no_truncation() {
        let input = "a\nb";
        let result = take_head(input, 5);
        assert_eq!(result, "a\nb");
    }

    #[test]
    fn test_take_tail() {
        let input = "a\nb\nc\nd\ne";
        let result = take_tail(input, 2);
        assert!(result.ends_with("d\ne"));
        assert!(result.contains("3 lines omitted"));
    }

    #[test]
    fn test_cap_lines() {
        let lines: Vec<String> = (0..20).map(|i| format!("line {}", i)).collect();
        let input = lines.join("\n");
        let result = cap_lines(&input, 10);
        assert!(result.contains("10 lines omitted"));
        // Head: 6 lines (60%), tail: 4 lines (40%)
        assert!(result.starts_with("line 0"));
        assert!(result.ends_with("line 19"));
    }

    #[test]
    fn test_cap_lines_no_truncation() {
        let input = "a\nb\nc";
        assert_eq!(cap_lines(input, 10), "a\nb\nc");
    }

    #[test]
    fn test_collapse_blank_lines() {
        let input = "a\n\n\n\nb\n\nc";
        let result = collapse_blank_lines(input);
        assert_eq!(result, "a\n\nb\n\nc");
    }

    #[test]
    fn test_full_pipeline() {
        let filter = CompiledFilter {
            name: "test".to_string(),
            match_regex: Regex::new(r"^cargo test").unwrap(),
            strip_ansi: true,
            replace_rules: vec![(Regex::new(r"^\s+Compiling .+$").unwrap(), String::new())],
            keep_lines: Some(Regex::new(r"^(test |error)").unwrap()),
            drop_lines: None,
            truncate_at: None,
            head: None,
            tail: None,
            max_lines: None,
            on_empty: Some("All tests passed".to_string()),
        };

        assert!(filter.matches("cargo test --release"));

        let input = "\x1b[32m   Compiling foo\x1b[0m\ntest bar ... ok\nerror: oops";
        let result = apply_filter(&filter, input);
        assert!(result.contains("test bar ... ok"));
        assert!(result.contains("error: oops"));
        assert!(!result.contains("Compiling"));
    }

    #[test]
    fn test_on_empty_triggers() {
        let filter = CompiledFilter {
            name: "test".to_string(),
            match_regex: Regex::new(r"test").unwrap(),
            strip_ansi: false,
            replace_rules: vec![],
            keep_lines: Some(Regex::new(r"^FAIL").unwrap()),
            drop_lines: None,
            truncate_at: None,
            head: None,
            tail: None,
            max_lines: None,
            on_empty: Some("All tests passed".to_string()),
        };

        let result = apply_filter(&filter, "test foo ... ok\ntest bar ... ok");
        assert_eq!(result, "All tests passed");
    }
}
