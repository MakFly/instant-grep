//! Pre-computed per-file metadata -- line offsets, symbols, smart summaries.
//! Stored in `.ig/filedata.bin` during index build.
//! Used by `read -s`, `context`, and `smart` commands for O(1) lookups.

use serde::{Deserialize, Serialize};

/// Pre-computed metadata for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileData {
    /// Byte offset of each line start (line 1 = offset 0, line 2 = first \n + 1, etc.)
    pub line_offsets: Vec<u32>,
    /// Extracted symbol definitions with enclosing block boundaries
    pub symbols: Vec<PrecomputedSymbol>,
    /// Smart summary line 1: role/purpose
    pub role: String,
    /// Smart summary line 2: public API
    pub public_api: String,
}

/// A pre-computed symbol definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrecomputedSymbol {
    /// Line number (1-indexed)
    pub line: u32,
    /// Symbol signature text (e.g., "pub fn main()")
    pub signature: String,
    /// Line number where the enclosing block ends (closing brace)
    pub block_end: u32,
}

/// Index of all file metadata, keyed by relative path.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileDataIndex {
    pub version: u32,
    pub entries: Vec<(String, FileData)>,
}

impl FileDataIndex {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }

    /// Look up metadata for a file by relative path.
    pub fn get(&self, rel_path: &str) -> Option<&FileData> {
        // Binary search since entries are sorted by path
        self.entries
            .binary_search_by_key(&rel_path, |(p, _)| p.as_str())
            .ok()
            .map(|idx| &self.entries[idx].1)
    }

    /// Load from .ig/filedata.bin
    pub fn load(ig_dir: &std::path::Path) -> Option<Self> {
        let path = ig_dir.join("filedata.bin");
        let bytes = std::fs::read(&path).ok()?;
        bincode::deserialize(&bytes).ok()
    }

    /// Save to .ig/filedata.bin
    pub fn save(&self, ig_dir: &std::path::Path) -> anyhow::Result<()> {
        let path = ig_dir.join("filedata.bin");
        let bytes = bincode::serialize(self)?;
        std::fs::write(&path, &bytes)?;
        Ok(())
    }
}

/// Compute line offsets for file content.
pub fn compute_line_offsets(content: &[u8]) -> Vec<u32> {
    let mut offsets = vec![0u32]; // line 1 starts at offset 0
    for (i, &byte) in content.iter().enumerate() {
        if byte == b'\n' && i + 1 < content.len() {
            offsets.push((i + 1) as u32);
        }
    }
    offsets
}

/// Extract symbols with block boundaries from file content.
/// Reuses the existing symbols::Lang and patterns infrastructure.
pub fn extract_symbols_with_boundaries(content: &str, ext: &str) -> Vec<PrecomputedSymbol> {
    use crate::symbols::Lang;

    let lang = Lang::from_ext(ext);
    let patterns = lang.patterns();
    if patterns.is_empty() {
        return Vec::new();
    }
    let sym_regex = match regex::Regex::new(patterns) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut symbols = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if sym_regex.is_match(line) {
            let line_num = (i + 1) as u32;
            let signature = line.trim().to_string();
            // Truncate very long signatures
            let signature = if signature.len() > 120 {
                let end = signature.floor_char_boundary(117);
                format!("{}...", &signature[..end])
            } else {
                signature
            };
            let block_end = find_block_end(&lines, i);
            symbols.push(PrecomputedSymbol {
                line: line_num,
                signature,
                block_end: block_end as u32,
            });
        }
    }

    symbols
}

/// Find the closing brace that matches the opening at or near `start_line`.
fn find_block_end(lines: &[&str], start_line: usize) -> usize {
    let mut depth: i32 = 0;
    let mut found_open = false;

    for i in start_line..lines.len() {
        for ch in lines[i].chars() {
            if ch == '{' {
                depth += 1;
                found_open = true;
            }
            if ch == '}' {
                depth -= 1;
            }
        }
        if found_open && depth <= 0 {
            return i + 1; // 1-indexed
        }
    }

    // For languages without braces (Python), use indentation
    if !found_open && start_line + 1 < lines.len() {
        let base_indent = lines[start_line].len() - lines[start_line].trim_start().len();
        for i in (start_line + 1)..lines.len() {
            let line = lines[i];
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            if indent <= base_indent {
                return i + 1; // 1-indexed
            }
        }
        return lines.len(); // until end of file
    }

    lines.len() // fallback: end of file
}

/// Extract a simple role description from file content.
/// Looks for the first doc comment or module-level comment.
pub fn extract_simple_role(content: &str, ext: &str) -> String {
    let comment_prefix = match ext {
        "rs" => Some(("//!", "///")),
        "py" | "pyi" => None, // handled separately via docstrings
        "go" => Some(("//", "//")),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(("//", "//")),
        "php" => Some(("//", "//")),
        _ => Some(("//", "//")),
    };

    // Try doc comments first
    if let Some((mod_prefix, _item_prefix)) = comment_prefix {
        for line in content.lines().take(20) {
            let trimmed = line.trim();
            if trimmed.starts_with(mod_prefix) {
                let text = trimmed[mod_prefix.len()..].trim();
                if !text.is_empty() && text.len() > 3 {
                    return truncate_str(text, 120);
                }
            }
            // Skip empty lines and shebangs at the top
            if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
                break;
            }
        }
    }

    // Python/Ruby: look for module docstring
    if matches!(ext, "py" | "pyi") {
        let mut in_docstring = false;
        for line in content.lines().take(20) {
            let trimmed = line.trim();
            if !in_docstring && (trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")) {
                if trimmed.len() > 3 && (trimmed.ends_with("\"\"\"") || trimmed.ends_with("'''")) {
                    let text = &trimmed[3..trimmed.len() - 3];
                    if !text.is_empty() {
                        return truncate_str(text, 120);
                    }
                }
                in_docstring = true;
                let text = &trimmed[3..];
                if !text.is_empty() {
                    return truncate_str(text, 120);
                }
                continue;
            }
            if in_docstring {
                if trimmed.ends_with("\"\"\"") || trimmed.ends_with("'''") {
                    break;
                }
                if !trimmed.is_empty() {
                    return truncate_str(trimmed, 120);
                }
            }
            // Skip empty lines, imports, encoding declarations
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("import")
                && !trimmed.starts_with("from")
            {
                break;
            }
        }
    }

    String::new()
}

/// Extract a simple public API summary from file content.
/// Returns first few public function/class/type signatures.
pub fn extract_simple_api(content: &str, ext: &str) -> String {
    use crate::symbols::Lang;

    let lang = Lang::from_ext(ext);
    let patterns = lang.patterns();
    if patterns.is_empty() {
        return String::new();
    }
    let re = match regex::Regex::new(patterns) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    let mut api_parts: Vec<String> = Vec::new();
    let max_items = 5;

    for line in content.lines() {
        if re.is_match(line) {
            let trimmed = line.trim();
            // Only include public/exported symbols
            let is_public = matches!(ext,
                "rs" if trimmed.starts_with("pub ") || trimmed.starts_with("pub("))
                || matches!(ext,
                    "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" if trimmed.starts_with("export "))
                || matches!(ext, "py" | "pyi" if !trimmed.starts_with('_'))
                || matches!(ext,
                    "go" if trimmed.starts_with("func ") && trimmed.chars().nth(5).is_some_and(|c| c.is_uppercase()))
                || matches!(ext, "go" if trimmed.starts_with("type ") && trimmed.chars().nth(5).is_some_and(|c| c.is_uppercase()))
                || matches!(ext,
                    "php" if trimmed.contains("public ") || !trimmed.contains("private ") && !trimmed.contains("protected "));

            if is_public {
                // Extract just the signature (up to opening brace or colon)
                let sig = trimmed
                    .find('{')
                    .map(|i| trimmed[..i].trim())
                    .unwrap_or(trimmed);
                let sig = if sig.len() > 80 { &sig[..77] } else { sig };
                api_parts.push(sig.to_string());
                if api_parts.len() >= max_items {
                    break;
                }
            }
        }
    }

    api_parts.join(", ")
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() > max {
        let end = s.floor_char_boundary(max - 3);
        format!("{}...", &s[..end])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_line_offsets() {
        let content = b"line1\nline2\nline3";
        let offsets = compute_line_offsets(content);
        assert_eq!(offsets, vec![0, 6, 12]);
    }

    #[test]
    fn test_compute_line_offsets_empty() {
        let offsets = compute_line_offsets(b"");
        assert_eq!(offsets, vec![0]);
    }

    #[test]
    fn test_filedata_index_lookup() {
        let mut idx = FileDataIndex::new();
        idx.entries.push((
            "src/main.rs".to_string(),
            FileData {
                line_offsets: vec![0, 10, 20],
                symbols: vec![],
                role: "entry point".to_string(),
                public_api: "fn main()".to_string(),
            },
        ));
        assert!(idx.get("src/main.rs").is_some());
        assert!(idx.get("src/lib.rs").is_none());
    }

    #[test]
    fn test_filedata_roundtrip() {
        let mut idx = FileDataIndex::new();
        idx.entries.push((
            "test.rs".to_string(),
            FileData {
                line_offsets: vec![0, 15, 30],
                symbols: vec![PrecomputedSymbol {
                    line: 2,
                    signature: "pub fn test()".to_string(),
                    block_end: 5,
                }],
                role: "test file".to_string(),
                public_api: "fn test()".to_string(),
            },
        ));
        let bytes = bincode::serialize(&idx).unwrap();
        let loaded: FileDataIndex = bincode::deserialize(&bytes).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].1.symbols[0].signature, "pub fn test()");
    }

    #[test]
    fn test_extract_symbols_with_boundaries_rust() {
        let content =
            "pub fn hello() {\n    println!(\"hi\");\n}\n\nstruct Foo {\n    x: i32,\n}\n";
        let syms = extract_symbols_with_boundaries(content, "rs");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].line, 1);
        assert!(syms[0].signature.contains("fn hello"));
        assert_eq!(syms[0].block_end, 3);
        assert_eq!(syms[1].line, 5);
        assert!(syms[1].signature.contains("struct Foo"));
        assert_eq!(syms[1].block_end, 7);
    }

    #[test]
    fn test_extract_symbols_python_indentation() {
        let content = "def greet(name):\n    return 'hello ' + name\n\ndef other():\n    pass\n";
        let syms = extract_symbols_with_boundaries(content, "py");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].line, 1);
        // Indentation-based: skips empty line 3, stops at line 4 (def other, indent 0 <= 0)
        assert_eq!(syms[0].block_end, 4);
    }

    #[test]
    fn test_extract_simple_role_rust() {
        let content = "//! Pre-computed per-file metadata.\n\nuse serde::Serialize;\n";
        let role = extract_simple_role(content, "rs");
        assert!(role.contains("Pre-computed"));
    }

    #[test]
    fn test_extract_simple_role_python() {
        let content = "\"\"\"Module for handling auth.\"\"\"\n\nimport os\n";
        let role = extract_simple_role(content, "py");
        assert!(role.contains("Module for handling auth"));
    }

    #[test]
    fn test_extract_simple_api_rust() {
        let content = "pub fn hello() {\n}\n\nfn private_fn() {\n}\n\npub struct Config {\n}\n";
        let api = extract_simple_api(content, "rs");
        assert!(api.contains("pub fn hello()"));
        assert!(api.contains("pub struct Config"));
        assert!(!api.contains("private_fn"));
    }

    #[test]
    fn test_find_block_end_nested() {
        let lines = vec![
            "pub fn foo() {",
            "    if true {",
            "        bar();",
            "    }",
            "}",
        ];
        let end = find_block_end(&lines, 0);
        assert_eq!(end, 5); // 1-indexed line 5
    }
}
