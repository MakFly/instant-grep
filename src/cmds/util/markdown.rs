//! Markdown body filter — strips noise (HTML comments, badges, image-only
//! lines, horizontal rules) and collapses multi-blank lines while preserving
//! fenced code blocks untouched.
//!
//! Ported from rtk's `gh_cmd::filter_markdown_body`. Used by `ig gh pr view`
//! and `ig gh issue view` (PR #5).

use regex::Regex;
use std::sync::OnceLock;

fn html_comment_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").unwrap())
}

fn badge_line_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^\s*\[!\[[^\]]*\]\([^)]*\)\]\([^)]*\)\s*$").unwrap())
}

fn image_only_line_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^\s*!\[[^\]]*\]\([^)]*\)\s*$").unwrap())
}

fn horizontal_rule_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^\s*(?:---+|\*\*\*+|___+)\s*$").unwrap())
}

fn multi_blank_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\n{3,}").unwrap())
}

/// Filter markdown body to remove noise while preserving meaningful content.
///
/// Strips HTML comments, badge lines (`[![alt](img)](url)`), image-only
/// lines, horizontal rules, and collapses runs of 3+ newlines to 2. Code
/// fences (``` and ~~~) are left untouched.
pub fn filter_markdown_body(body: &str) -> String {
    if body.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let mut remaining = body;

    loop {
        let fence_pos = remaining
            .find("```")
            .or_else(|| remaining.find("~~~"))
            .map(|pos| {
                let fence = if remaining[pos..].starts_with("```") {
                    "```"
                } else {
                    "~~~"
                };
                (pos, fence)
            });

        match fence_pos {
            Some((start, fence)) => {
                let before = &remaining[..start];
                result.push_str(&filter_segment(before));

                let after_open = start + fence.len();
                let code_start = remaining[after_open..]
                    .find('\n')
                    .map(|p| after_open + p + 1)
                    .unwrap_or(remaining.len());

                let close_pos = remaining[code_start..]
                    .find(fence)
                    .map(|p| code_start + p + fence.len());

                match close_pos {
                    Some(end) => {
                        result.push_str(&remaining[start..end]);
                        let after_close = remaining[end..]
                            .find('\n')
                            .map(|p| end + p + 1)
                            .unwrap_or(remaining.len());
                        result.push_str(&remaining[end..after_close]);
                        remaining = &remaining[after_close..];
                    }
                    None => {
                        result.push_str(&remaining[start..]);
                        remaining = "";
                    }
                }
            }
            None => {
                result.push_str(&filter_segment(remaining));
                break;
            }
        }
    }

    result.trim().to_string()
}

fn filter_segment(text: &str) -> String {
    let mut s = html_comment_re().replace_all(text, "").to_string();
    s = badge_line_re().replace_all(&s, "").to_string();
    s = image_only_line_re().replace_all(&s, "").to_string();
    s = horizontal_rule_re().replace_all(&s, "").to_string();
    s = multi_blank_re().replace_all(&s, "\n\n").to_string();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_comment_single_line() {
        assert_eq!(
            filter_markdown_body("hello <!-- skip --> world"),
            "hello  world"
        );
    }

    #[test]
    fn strips_html_comment_multiline() {
        let input = "before\n<!--\nblock\ncomment\n-->\nafter";
        let out = filter_markdown_body(input);
        assert!(out.contains("before"));
        assert!(out.contains("after"));
        assert!(!out.contains("block"));
    }

    #[test]
    fn strips_badge_line() {
        let input = "title\n\n[![CI](https://img.shields.io/x)](https://example.com)\n\nbody";
        let out = filter_markdown_body(input);
        assert!(out.contains("title"));
        assert!(out.contains("body"));
        assert!(!out.contains("shields.io"));
    }

    #[test]
    fn strips_image_only_line() {
        let input = "alpha\n![logo](foo.png)\nomega";
        let out = filter_markdown_body(input);
        assert!(!out.contains("foo.png"));
    }

    #[test]
    fn strips_horizontal_rule() {
        let input = "a\n\n---\n\nb";
        let out = filter_markdown_body(input);
        assert!(!out.contains("---"));
    }

    #[test]
    fn collapses_multi_blank() {
        let input = "a\n\n\n\n\nb";
        let out = filter_markdown_body(input);
        assert_eq!(out, "a\n\nb");
    }

    #[test]
    fn preserves_code_fence() {
        let input = "before\n\n```\n<!-- not stripped -->\n![not stripped](x)\n```\n\nafter";
        let out = filter_markdown_body(input);
        assert!(out.contains("<!-- not stripped -->"));
        assert!(out.contains("![not stripped](x)"));
    }

    #[test]
    fn preserves_tilde_fence() {
        let input = "~~~\n<!-- raw -->\n~~~";
        assert!(filter_markdown_body(input).contains("<!-- raw -->"));
    }

    #[test]
    fn empty_string() {
        assert_eq!(filter_markdown_body(""), "");
    }
}
