use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::index::filedata::FileData;
use crate::symbols::Lang;
use crate::util::{is_binary, is_preamble_line};

pub struct ReadResult {
    pub file: String,
    pub lines: Vec<(usize, String)>,
}

/// Filter level for file reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterLevel {
    /// No filtering — return all lines.
    Full,
    /// Signatures mode — imports + symbol definitions only.
    Signatures,
    /// Aggressive mode — strip comments, collapse blanks, elide function bodies.
    Aggressive,
}

/// Read a file and return numbered lines.
/// In signatures mode, only return import lines and symbol definitions.
#[allow(dead_code)]
pub fn read_file(file: &Path, signatures_only: bool) -> Result<ReadResult> {
    let level = if signatures_only {
        FilterLevel::Signatures
    } else {
        FilterLevel::Full
    };
    read_file_filtered(file, level)
}

/// Read a file with a specific filter level.
pub fn read_file_filtered(file: &Path, level: FilterLevel) -> Result<ReadResult> {
    let content = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;

    if is_binary(&content) {
        bail!("binary file: {}", file.display());
    }

    let text = std::str::from_utf8(&content)
        .with_context(|| format!("{} is not valid UTF-8", file.display()))?;

    let file_str = file.to_string_lossy().to_string();

    match level {
        FilterLevel::Full => {
            let lines: Vec<(usize, String)> = text
                .lines()
                .enumerate()
                .map(|(i, line)| (i + 1, line.to_string()))
                .collect();
            Ok(ReadResult {
                file: file_str,
                lines,
            })
        }
        FilterLevel::Signatures => read_signatures(text, file, file_str),
        FilterLevel::Aggressive => read_aggressive(text, file, file_str),
    }
}

/// Signatures mode: imports + symbol definitions only.
fn read_signatures(text: &str, file: &Path, file_str: String) -> Result<ReadResult> {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Lang::from_ext(ext);
    let sym_regex = if !lang.patterns().is_empty() {
        regex::Regex::new(lang.patterns()).ok()
    } else {
        None
    };

    let mut lines = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Keep imports
        if is_preamble_line(trimmed) {
            lines.push((i + 1, line.to_string()));
            continue;
        }

        // Keep symbol definitions
        if let Some(ref re) = sym_regex
            && re.is_match(line)
        {
            // Strip trailing opening brace for cleaner output
            let clean = line.trim_end().trim_end_matches('{').trim_end();
            lines.push((i + 1, clean.to_string()));
        }
    }

    Ok(ReadResult {
        file: file_str,
        lines,
    })
}

/// Signatures mode using pre-computed FileData (O(1) lookup).
/// Falls back to regex-based extraction if filedata is unavailable.
pub fn read_signatures_cached(file: &Path, filedata: &FileData) -> Result<ReadResult> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let file_str = file.to_string_lossy().to_string();

    let mut result_lines = Vec::new();

    // Add import lines (preamble)
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if is_preamble_line(trimmed) {
            result_lines.push((i + 1, line.to_string()));
        }
    }

    // Add symbol signatures from pre-computed data
    for sym in &filedata.symbols {
        let line_idx = (sym.line as usize).saturating_sub(1);
        if line_idx < lines.len() {
            let clean = lines[line_idx].trim_end().trim_end_matches('{').trim_end();
            result_lines.push((sym.line as usize, clean.to_string()));
        }
    }

    // Sort by line number and dedup
    result_lines.sort_by_key(|&(n, _)| n);
    result_lines.dedup_by_key(|e| e.0);

    Ok(ReadResult {
        file: file_str,
        lines: result_lines,
    })
}

/// Aggressive mode: strip comments, collapse blanks, elide function bodies.
///
/// Strategy:
/// 1. Strip all comments (single-line and multi-line, language-aware)
/// 2. Collapse consecutive blank lines to one
/// 3. Keep imports/use statements
/// 4. Keep struct/class/enum/interface/type definitions WITH their fields
/// 5. Keep function/method signatures but replace bodies with `// ...`
/// 6. Strip long string literals (replace with `"..."`)
/// 7. Keep trait implementations, impl blocks headers
fn read_aggressive(text: &str, file: &Path, file_str: String) -> Result<ReadResult> {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Lang::from_ext(ext);
    let sym_regex = if !lang.patterns().is_empty() {
        regex::Regex::new(lang.patterns()).ok()
    } else {
        None
    };

    let comment_info = CommentSyntax::for_lang(lang);
    let source_lines: Vec<&str> = text.lines().collect();

    // Phase 1: strip comments and identify line roles
    let mut stripped_lines: Vec<(usize, String, LineRole)> = Vec::new();
    let mut in_block_comment = false;
    let mut in_python_docstring = false;
    let mut python_docstring_delim: &str = "";
    let mut in_ruby_block = false;

    for (i, &line) in source_lines.iter().enumerate() {
        let trimmed = line.trim();

        // Handle Ruby =begin/=end block comments
        if comment_info.has_ruby_block {
            if in_ruby_block {
                if trimmed == "=end" {
                    in_ruby_block = false;
                }
                continue;
            }
            if trimmed == "=begin" {
                in_ruby_block = true;
                continue;
            }
        }

        // Handle Python triple-quote docstrings
        if comment_info.has_python_docstring {
            if in_python_docstring {
                if trimmed.contains(python_docstring_delim) {
                    in_python_docstring = false;
                }
                continue;
            }
            if (trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''"))
                && !trimmed[3..].contains(&trimmed[..3])
            {
                python_docstring_delim = if trimmed.starts_with("\"\"\"") {
                    "\"\"\""
                } else {
                    "'''"
                };
                in_python_docstring = true;
                continue;
            }
            // Single-line docstring (opens and closes on same line)
            if (trimmed.starts_with("\"\"\"") && trimmed[3..].contains("\"\"\""))
                || (trimmed.starts_with("'''") && trimmed[3..].contains("'''"))
            {
                continue;
            }
        }

        // Handle multi-line block comments (/* */)
        if comment_info.block_start.is_some() {
            if in_block_comment {
                if trimmed.contains("*/") {
                    in_block_comment = false;
                }
                continue;
            }
            if trimmed.starts_with("/*") || trimmed.starts_with("/**") {
                if !trimmed.contains("*/") {
                    in_block_comment = true;
                }
                continue;
            }
        }

        // Strip single-line comments
        let mut cleaned = line.to_string();
        let is_comment_only = comment_info
            .line_prefixes
            .iter()
            .any(|p| trimmed.starts_with(p))
            // PHP 8 attributes start with #[ — don't treat as # comment
            && !(matches!(lang, Lang::Php) && trimmed.starts_with("#["));

        if is_comment_only {
            // Keep doc comments in Rust (/// and //!) as they're part of the API
            if matches!(lang, Lang::Rust)
                && (trimmed.starts_with("///") || trimmed.starts_with("//!"))
            {
                // Keep doc comments but mark as doc
            } else {
                continue;
            }
        }

        // Strip inline comments (be careful not to strip inside strings)
        if !is_comment_only {
            cleaned = strip_inline_comment(&cleaned, &comment_info);
        }

        // Strip long string literals
        cleaned = strip_long_strings(&cleaned);

        let role = classify_line(trimmed, &sym_regex, lang);
        stripped_lines.push((i + 1, cleaned, role));
    }

    // Phase 2: process function bodies — elide with `// ...`
    let mut result_lines: Vec<(usize, String)> = Vec::new();
    let mut brace_depth: i32 = 0;
    let mut in_fn_body = false;
    let mut fn_body_start_depth: i32 = 0;
    let mut last_was_blank = false;
    let mut elided = false;
    // Track struct/enum/class body to keep fields
    let mut in_definition_body = false;
    let mut def_body_start_depth: i32 = 0;
    // Track pending function signature (for PSR-style { on next line)
    let mut pending_fn_elide = false;
    let mut pending_def_body = false;
    // Track whether we were inside a definition body before entering a nested fn body
    let mut was_in_definition_body = false;
    let mut saved_def_body_depth: i32 = 0;

    for (line_num, line, role) in &stripped_lines {
        let trimmed = line.trim();

        // Track brace depth
        let open_braces = trimmed.chars().filter(|&c| c == '{').count() as i32;
        let close_braces = trimmed.chars().filter(|&c| c == '}').count() as i32;

        // Handle PSR-style: { on its own line after a function/class signature
        if pending_fn_elide && trimmed == "{" {
            pending_fn_elide = false;
            in_fn_body = true;
            fn_body_start_depth = brace_depth;
            brace_depth += 1;
            continue;
        } else if pending_fn_elide {
            // Not a lone {, cancel pending
            pending_fn_elide = false;
        }
        if pending_def_body && trimmed == "{" {
            pending_def_body = false;
            in_definition_body = true;
            def_body_start_depth = brace_depth;
            brace_depth += 1;
            continue;
        } else if pending_def_body {
            pending_def_body = false;
        }

        match role {
            LineRole::Import => {
                in_fn_body = false;
                in_definition_body = false;
                last_was_blank = false;
                result_lines.push((*line_num, line.clone()));
                brace_depth += open_braces - close_braces;
                continue;
            }
            LineRole::SymbolDef => {
                // Check if we're inside a definition body (class/struct/enum)
                if in_definition_body {
                    // Nested symbol inside a class body
                    last_was_blank = false;
                    elided = false;
                    result_lines.push((*line_num, line.clone()));

                    if is_function_def(trimmed, lang) {
                        // Method inside class — keep signature, elide body
                        if trimmed.contains('{') && open_braces > close_braces {
                            let is_oneliner = open_braces > 0 && open_braces == close_braces;
                            if !is_oneliner {
                                was_in_definition_body = true;
                                saved_def_body_depth = def_body_start_depth;
                                in_fn_body = true;
                                in_definition_body = false;
                                fn_body_start_depth = brace_depth;
                                elided = false;
                            }
                        } else if !trimmed.contains('{') {
                            // PSR-style: { on next line
                            was_in_definition_body = true;
                            saved_def_body_depth = def_body_start_depth;
                            pending_fn_elide = true;
                        }
                    }
                    // else: nested type def (inner class/enum) — just keep the line, stay in definition_body

                    brace_depth += open_braces - close_braces;
                    continue;
                }

                // === Top-level SymbolDef handling ===
                in_fn_body = false;
                in_definition_body = false;
                was_in_definition_body = false;
                last_was_blank = false;
                elided = false;

                // Determine if this is a function or a type definition
                if is_function_def(trimmed, lang) {
                    // Function: keep signature, start eliding body
                    result_lines.push((*line_num, line.clone()));
                    // One-liner: { and } on same line — don't elide (e.g. PHP scopes)
                    let is_oneliner = open_braces > 0 && open_braces == close_braces;
                    if !is_oneliner && trimmed.contains('{') && open_braces > close_braces {
                        in_fn_body = true;
                        fn_body_start_depth = brace_depth;
                    } else if !trimmed.contains('{') {
                        // PSR-style: { will be on the next line
                        pending_fn_elide = true;
                    }
                } else {
                    // Struct/enum/class/trait/interface/impl: keep with fields
                    result_lines.push((*line_num, line.clone()));
                    if trimmed.contains('{') && open_braces > close_braces {
                        in_definition_body = true;
                        def_body_start_depth = brace_depth;
                    } else if !trimmed.contains('{') {
                        // PSR-style: { on next line for class/trait/etc.
                        pending_def_body = true;
                    }
                }
                brace_depth += open_braces - close_braces;
                continue;
            }
            LineRole::Code | LineRole::Blank => {}
        }

        brace_depth += open_braces - close_braces;

        // Inside a function body: elide
        if in_fn_body {
            if brace_depth <= fn_body_start_depth {
                in_fn_body = false;
                // Restore definition body tracking if we were inside a class
                if was_in_definition_body {
                    in_definition_body = true;
                    def_body_start_depth = saved_def_body_depth;
                    was_in_definition_body = false;
                }
                // Emit closing brace
                result_lines.push((*line_num, line.clone()));
                elided = false;
            } else if !elided {
                // Emit single elision marker
                let indent = line.len() - line.trim_start().len();
                let spaces = " ".repeat(indent + 4);
                result_lines.push((*line_num, format!("{}// ...", spaces)));
                elided = true;
            }
            continue;
        }

        // Inside a definition body (struct/enum/class): keep fields but elide nested function bodies
        if in_definition_body {
            if brace_depth <= def_body_start_depth {
                in_definition_body = false;
                result_lines.push((*line_num, line.clone()));
            } else {
                // Check if this line is a nested function definition (method inside class)
                let nested_role = classify_line(trimmed, &sym_regex, lang);
                if matches!(nested_role, LineRole::SymbolDef) && is_function_def(trimmed, lang) {
                    // Found a method inside the class — keep signature, start eliding its body
                    result_lines.push((*line_num, line.clone()));
                    if trimmed.contains('{') && open_braces > close_braces {
                        let is_oneliner = open_braces > 0 && open_braces == close_braces;
                        if !is_oneliner {
                            // Save definition body state before entering nested fn body
                            was_in_definition_body = true;
                            saved_def_body_depth = def_body_start_depth;
                            in_fn_body = true;
                            in_definition_body = false;
                            fn_body_start_depth = brace_depth - open_braces + close_braces;
                            elided = false;
                        }
                    } else if !trimmed.contains('{') {
                        // PSR-style: { on next line
                        was_in_definition_body = true;
                        saved_def_body_depth = def_body_start_depth;
                        pending_fn_elide = true;
                    }
                    last_was_blank = false;
                } else if !trimmed.is_empty() {
                    // Keep field lines, skip blank lines inside
                    result_lines.push((*line_num, line.clone()));
                    last_was_blank = false;
                } else if !last_was_blank {
                    result_lines.push((*line_num, String::new()));
                    last_was_blank = true;
                }
            }
            continue;
        }

        // Normal code line
        if trimmed.is_empty() {
            if !last_was_blank {
                result_lines.push((*line_num, String::new()));
                last_was_blank = true;
            }
        } else {
            last_was_blank = false;
            result_lines.push((*line_num, line.clone()));
        }
    }

    Ok(ReadResult {
        file: file_str,
        lines: result_lines,
    })
}

#[derive(Debug)]
enum LineRole {
    Import,
    SymbolDef,
    Code,
    Blank,
}

fn classify_line(trimmed: &str, sym_regex: &Option<regex::Regex>, lang: Lang) -> LineRole {
    if trimmed.is_empty() {
        return LineRole::Blank;
    }
    // In PHP, #[...] is an attribute, not a preamble/import
    let is_php_attribute = matches!(lang, Lang::Php) && trimmed.starts_with("#[");
    if !is_php_attribute && is_preamble_line(trimmed) {
        return LineRole::Import;
    }
    if let Some(re) = sym_regex {
        if re.is_match(trimmed) {
            return LineRole::SymbolDef;
        }
    }
    // PHP-specific: recognize Laravel model properties and relation returns as definitions
    if matches!(lang, Lang::Php) {
        if is_php_structural_line(trimmed) {
            return LineRole::SymbolDef;
        }
    }
    LineRole::Code
}

/// Recognize PHP/Laravel structural lines that should be kept in aggressive mode.
fn is_php_structural_line(trimmed: &str) -> bool {
    // Model config properties: protected $fillable = [...], $casts, $hidden, $table, etc.
    if trimmed.starts_with("protected $")
        || trimmed.starts_with("public $")
        || trimmed.starts_with("private $")
    {
        let config_props = [
            "$table",
            "$fillable",
            "$hidden",
            "$guarded",
            "$casts",
            "$appends",
            "$with",
            "$connection",
            "$primaryKey",
            "$timestamps",
            "$perPage",
            "$incrementing",
            "$signature",
            "$description",
            "$dates",
        ];
        if config_props.iter().any(|p| trimmed.contains(p)) {
            return true;
        }
    }
    // Eloquent relations: return $this->hasMany(...), belongsTo(...), etc.
    if trimmed.starts_with("return $this->") {
        let relations = [
            "hasMany(",
            "hasOne(",
            "belongsTo(",
            "belongsToMany(",
            "hasManyThrough(",
            "hasOneThrough(",
            "morphTo(",
            "morphMany(",
            "morphOne(",
            "morphToMany(",
        ];
        if relations.iter().any(|r| trimmed.contains(r)) {
            return true;
        }
    }
    // Schema operations: Schema::create(...), Schema::table(...)
    if trimmed.starts_with("Schema::") {
        return true;
    }
    // Route definitions: Route::get(...), Route::post(...), etc.
    if trimmed.starts_with("Route::") {
        return true;
    }
    false
}

/// Check if a symbol definition line is a function (vs struct/enum/class/trait).
fn is_function_def(trimmed: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust => {
            // fn or async fn, but not struct/enum/trait/impl/type/mod/const/static
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            words
                .iter()
                .any(|w| *w == "fn" || w.starts_with("fn(") || w.starts_with("fn<"))
                && !words.iter().any(|w| {
                    *w == "struct"
                        || *w == "enum"
                        || *w == "trait"
                        || *w == "impl"
                        || *w == "type"
                        || *w == "mod"
                })
        }
        Lang::Python => trimmed.contains("def ") && !trimmed.contains("class "),
        Lang::Go => trimmed.starts_with("func "),
        Lang::TypeScript | Lang::JavaScript => {
            (trimmed.contains("function ") || trimmed.contains("function*("))
                && !trimmed.contains("class ")
                && !trimmed.contains("interface ")
                && !trimmed.contains("type ")
                && !trimmed.contains("enum ")
        }
        Lang::Php => {
            // Must contain "function " and not be a class/interface/trait/enum/const/case/attribute
            trimmed.contains("function ")
                && !trimmed.contains("class ")
                && !trimmed.contains("interface ")
                && !trimmed.contains("trait ")
                && !trimmed.contains("enum ")
                // Not a const, case, property, attribute, namespace, or declare
                && !trimmed.starts_with("const ")
                && !trimmed.starts_with("case ")
                && !trimmed.starts_with("#[")
                && !trimmed.starts_with("namespace ")
                && !trimmed.starts_with("declare")
        }
        Lang::Other => {
            trimmed.contains("function ")
                || trimmed.contains("def ")
                || (trimmed.contains("fn ")
                    && !trimmed.contains("struct ")
                    && !trimmed.contains("class "))
        }
    }
}

struct CommentSyntax {
    line_prefixes: Vec<&'static str>,
    block_start: Option<&'static str>,
    has_python_docstring: bool,
    has_ruby_block: bool,
}

impl CommentSyntax {
    fn for_lang(lang: Lang) -> Self {
        match lang {
            Lang::Rust => Self {
                line_prefixes: vec!["//"],
                block_start: Some("/*"),
                has_python_docstring: false,
                has_ruby_block: false,
            },
            Lang::TypeScript | Lang::JavaScript => Self {
                line_prefixes: vec!["//"],
                block_start: Some("/*"),
                has_python_docstring: false,
                has_ruby_block: false,
            },
            Lang::Python => Self {
                line_prefixes: vec!["#"],
                block_start: None,
                has_python_docstring: true,
                has_ruby_block: false,
            },
            Lang::Go => Self {
                line_prefixes: vec!["//"],
                block_start: Some("/*"),
                has_python_docstring: false,
                has_ruby_block: false,
            },
            Lang::Php => Self {
                line_prefixes: vec!["//", "#"],
                block_start: Some("/*"),
                has_python_docstring: false,
                has_ruby_block: false,
            },
            Lang::Other => Self {
                line_prefixes: vec!["//", "#"],
                block_start: Some("/*"),
                has_python_docstring: false,
                has_ruby_block: false,
            },
        }
    }
}

/// Strip inline comments (after code), being careful about strings.
fn strip_inline_comment(line: &str, info: &CommentSyntax) -> String {
    // Simple heuristic: find comment markers not inside quotes
    let bytes = line.as_bytes();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            i += 2; // skip escaped char
            continue;
        }
        if b == b'\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if b == b'"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        }

        if !in_single_quote && !in_double_quote {
            for prefix in &info.line_prefixes {
                if line[i..].starts_with(prefix) {
                    return line[..i].trim_end().to_string();
                }
            }
        }
        i += 1;
    }

    line.to_string()
}

/// Replace long string literals with `"..."`.
fn strip_long_strings(line: &str) -> String {
    // Only strip strings longer than 40 chars
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1; // skip escaped
                }
                i += 1;
            }
            if i < chars.len() {
                i += 1; // closing quote
            }
            let len = i - start;
            if len > 42 {
                result.push(quote);
                result.push_str("...");
                result.push(quote);
            } else {
                for &c in &chars[start..i] {
                    result.push(c);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_full() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file(f.path(), false).unwrap();
        assert_eq!(result.lines.len(), 5);
        assert_eq!(result.lines[0].0, 1);
        assert!(result.lines[0].1.contains("use std::io"));
    }

    #[test]
    fn test_read_signatures() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "use std::path::Path;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "pub fn main() {{").unwrap();
        writeln!(f, "    let x = 42;").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "struct Config {{").unwrap();
        writeln!(f, "    name: String,").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file(f.path(), true).unwrap();
        // Should have: 2 imports + 2 symbols = 4 lines
        assert_eq!(result.lines.len(), 4, "got: {:?}", result.lines);
        assert!(result.lines[0].1.contains("use std::io"));
        assert!(result.lines[1].1.contains("use std::path"));
        assert!(result.lines[2].1.contains("pub fn main"));
        assert!(result.lines[3].1.contains("struct Config"));
    }

    #[test]
    fn test_read_binary_file_errors() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[0u8, 1, 2, 0, 3]).unwrap();

        let result = read_file(f.path(), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_nonexistent_file_errors() {
        let result = read_file(Path::new("/nonexistent/file.rs"), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_typescript_signatures() {
        let mut f = NamedTempFile::with_suffix(".ts").unwrap();
        writeln!(f, "import {{ useState }} from 'react';").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "export function greet(name: string) {{").unwrap();
        writeln!(f, "  return `hello ${{name}}`;").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "export class UserService {{").unwrap();
        writeln!(f, "  private name: string;").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file(f.path(), true).unwrap();
        assert!(result.lines.len() >= 3, "got: {:?}", result.lines);
        assert!(result.lines[0].1.contains("import"));
        assert!(
            result
                .lines
                .iter()
                .any(|(_, l)| l.contains("function greet"))
        );
        assert!(
            result
                .lines
                .iter()
                .any(|(_, l)| l.contains("class UserService"))
        );
    }

    #[test]
    fn test_aggressive_strips_comments() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "// This is a comment").unwrap();
        writeln!(f, "/* Block comment */").unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    // inner comment").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("This is a comment"));
        assert!(!text.contains("Block comment"));
        assert!(!text.contains("inner comment"));
        assert!(text.contains("use std::io"));
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn test_aggressive_elides_function_bodies() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "pub fn compute(x: i32) -> i32 {{").unwrap();
        writeln!(f, "    let a = x * 2;").unwrap();
        writeln!(f, "    let b = a + 1;").unwrap();
        writeln!(f, "    let c = b.pow(2);").unwrap();
        writeln!(f, "    c").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("pub fn compute(x: i32) -> i32 {"));
        assert!(text.contains("// ..."));
        assert!(!text.contains("let a = x * 2"));
        assert!(!text.contains("let b = a + 1"));
    }

    #[test]
    fn test_aggressive_keeps_struct_fields() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "pub struct Config {{").unwrap();
        writeln!(f, "    pub name: String,").unwrap();
        writeln!(f, "    pub value: i32,").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("pub struct Config {"));
        assert!(text.contains("pub name: String"));
        assert!(text.contains("pub value: i32"));
    }

    #[test]
    fn test_aggressive_collapses_blank_lines() {
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "use std::path::Path;").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        // Multiple blanks should collapse to at most one
        let blank_count = result
            .lines
            .iter()
            .filter(|(_, l)| l.trim().is_empty())
            .count();
        assert!(
            blank_count <= 1,
            "expected at most 1 blank line, got {}",
            blank_count
        );
        // Should have imports and at most one blank between them
        assert!(
            result.lines.len() <= 5,
            "expected compact output, got {} lines",
            result.lines.len()
        );
    }

    #[test]
    fn test_aggressive_strips_long_strings() {
        let long_str = "a".repeat(50);
        let line = format!("    let x = \"{}\";", long_str);
        let stripped = strip_long_strings(&line);
        assert!(
            stripped.contains("\"...\""),
            "long string should be replaced, got: {}",
            stripped
        );
    }

    #[test]
    fn test_aggressive_python_comments() {
        let mut f = NamedTempFile::with_suffix(".py").unwrap();
        writeln!(f, "import os").unwrap();
        writeln!(f, "# A comment").unwrap();
        writeln!(f, "\"\"\"").unwrap();
        writeln!(f, "This is a docstring").unwrap();
        writeln!(f, "that spans multiple lines").unwrap();
        writeln!(f, "\"\"\"").unwrap();
        writeln!(f, "def hello():").unwrap();
        writeln!(f, "    print('hello')").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("import os"));
        assert!(!text.contains("A comment"));
        assert!(!text.contains("docstring"));
        assert!(text.contains("def hello()"));
    }

    #[test]
    fn test_aggressive_backward_compat() {
        // read_file with signatures_only=false should still work as Full
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        writeln!(f, "// comment").unwrap();
        writeln!(f, "fn main() {{}}").unwrap();

        let result = read_file(f.path(), false).unwrap();
        assert_eq!(result.lines.len(), 2);
        assert!(result.lines[0].1.contains("// comment"));
    }

    #[test]
    fn test_filter_level_enum() {
        assert_ne!(FilterLevel::Full, FilterLevel::Signatures);
        assert_ne!(FilterLevel::Full, FilterLevel::Aggressive);
        assert_ne!(FilterLevel::Signatures, FilterLevel::Aggressive);
    }

    #[test]
    fn test_aggressive_php_class_methods() {
        let mut f = NamedTempFile::with_suffix(".php").unwrap();
        writeln!(f, "<?php").unwrap();
        writeln!(f, "namespace App\\Controllers;").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "class ClientController {{").unwrap();
        writeln!(f, "    public function index() {{").unwrap();
        writeln!(f, "        $clients = Client::all();").unwrap();
        writeln!(
            f,
            "        return view('clients.index', compact('clients'));"
        )
        .unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "    public function show($id) {{").unwrap();
        writeln!(f, "        $client = Client::findOrFail($id);").unwrap();
        writeln!(f, "        return view('clients.show', compact('client'));").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        // Should keep class declaration and method signatures
        assert!(
            text.contains("class ClientController"),
            "class declaration missing"
        );
        assert!(
            text.contains("public function index()"),
            "method signature missing"
        );
        assert!(
            text.contains("public function show($id)"),
            "method signature missing"
        );

        // Should elide method bodies
        assert!(
            !text.contains("Client::all()"),
            "method body should be elided"
        );
        assert!(
            !text.contains("Client::findOrFail"),
            "method body should be elided"
        );
        assert!(text.contains("// ..."), "should have elision marker");
    }

    #[test]
    fn test_aggressive_ts_class_methods() {
        let mut f = NamedTempFile::with_suffix(".ts").unwrap();
        writeln!(f, "import {{ Injectable }} from '@nestjs/common';").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "export class UserService {{").unwrap();
        writeln!(f, "    private users: User[] = [];").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "    function findAll(): User[] {{").unwrap();
        writeln!(f, "        return this.users.filter(u => u.active);").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "    function findById(id: string): User {{").unwrap();
        writeln!(f, "        const user = this.users.find(u => u.id === id);").unwrap();
        writeln!(f, "        if (!user) throw new Error('Not found');").unwrap();
        writeln!(f, "        return user;").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "}}").unwrap();

        let result = read_file_filtered(f.path(), FilterLevel::Aggressive).unwrap();
        let text: String = result
            .lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            text.contains("class UserService"),
            "class declaration missing"
        );
        assert!(
            text.contains("private users: User[]"),
            "field should be kept"
        );
        assert!(text.contains("function findAll()"), "method sig missing");
        assert!(
            text.contains("function findById(id: string)"),
            "method sig missing"
        );
        assert!(!text.contains("this.users.filter"), "body should be elided");
        assert!(!text.contains("this.users.find"), "body should be elided");
    }
}
