use std::path::Path;

use anyhow::Result;

use crate::index::filedata::FileData;
use crate::symbols::{self, Lang};
use crate::util::{is_binary, is_preamble_line};
use crate::walk;

pub struct SmartSummary {
    pub file: String,
    pub role: String,       // L1: what this file does
    pub public_api: String, // L2: exported/public symbols
}

/// A directory-level aggregate summary — one output block for the whole tree
/// instead of a per-file report. Used in compact (pipe) mode on big dirs,
/// where per-file output is O(N) with N in the thousands.
pub struct DirAggregate {
    pub total_files: usize,
    pub dir_count: usize,
    /// (extension, file_count) sorted by count desc, top 6
    pub top_exts: Vec<(String, usize)>,
    /// (top-level subdir, file_count) sorted desc, top 8
    pub top_subdirs: Vec<(String, usize)>,
    /// Well-known manifest / entry files found at the root (package.json, Cargo.toml, …)
    pub key_files: Vec<String>,
}

/// Fast directory aggregate: no file content reads, just walks and counts.
pub fn smart_dir_aggregate(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<DirAggregate> {
    use std::collections::{BTreeMap, BTreeSet};
    let files = walk::walk_files(
        root,
        use_default_excludes,
        max_file_size,
        type_filter,
        glob_filter,
    )?;
    let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut subdir_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut dirs: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    let mut key_files: Vec<String> = Vec::new();
    const MANIFESTS: &[&str] = &[
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "composer.json",
        "requirements.txt",
        "Gemfile",
        "pom.xml",
        "build.gradle",
        "README.md",
        "readme.md",
        "Dockerfile",
        "docker-compose.yml",
        "tsconfig.json",
        "Makefile",
    ];
    for path in &files {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if let Some(parent) = rel.parent() {
            dirs.insert(parent.to_path_buf());
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("(none)")
            .to_string();
        *ext_counts.entry(ext).or_default() += 1;
        if let Some(first) = rel.components().next()
            && let Some(s) = first.as_os_str().to_str()
            && !s.is_empty()
            && rel.components().count() > 1
        {
            *subdir_counts.entry(s.to_string()).or_default() += 1;
        }
        if let Some(name) = rel.file_name().and_then(|s| s.to_str())
            && MANIFESTS.iter().any(|m| m.eq_ignore_ascii_case(name))
            && rel.components().count() <= 3
        {
            key_files.push(rel.to_string_lossy().to_string());
        }
    }
    let mut top_exts: Vec<(String, usize)> = ext_counts.into_iter().collect();
    top_exts.sort_by(|a, b| b.1.cmp(&a.1));
    top_exts.truncate(6);
    let mut top_subdirs: Vec<(String, usize)> = subdir_counts.into_iter().collect();
    top_subdirs.sort_by(|a, b| b.1.cmp(&a.1));
    top_subdirs.truncate(8);
    key_files.sort();
    key_files.dedup();
    key_files.truncate(10);
    Ok(DirAggregate {
        total_files: files.len(),
        dir_count: dirs.len(),
        top_exts,
        top_subdirs,
        key_files,
    })
}

/// Generate 2-line smart summaries for all files under a path.
pub fn smart_summarize(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<Vec<SmartSummary>> {
    let files = walk::walk_files(
        root,
        use_default_excludes,
        max_file_size,
        type_filter,
        glob_filter,
    )?;
    let mut summaries = Vec::new();

    for path in &files {
        let content = match std::fs::read(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if is_binary(&content) {
            continue;
        }

        let text = match std::str::from_utf8(&content) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let rel_path = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Lang::from_ext(ext);

        let role = extract_role(text, lang);
        let public_api = extract_public_api(text, &rel_str, lang);

        summaries.push(SmartSummary {
            file: rel_str,
            role,
            public_api,
        });
    }

    Ok(summaries)
}

/// Generate a smart summary for a single file.
pub fn smart_summarize_file(file: &Path, root: &Path) -> Result<SmartSummary> {
    let content =
        std::fs::read(file).map_err(|e| anyhow::anyhow!("reading {}: {}", file.display(), e))?;

    if is_binary(&content) {
        let rel = file.strip_prefix(root).unwrap_or(file);
        return Ok(SmartSummary {
            file: rel.to_string_lossy().to_string(),
            role: "binary file".to_string(),
            public_api: String::new(),
        });
    }

    let text =
        std::str::from_utf8(&content).map_err(|e| anyhow::anyhow!("{}: {}", file.display(), e))?;

    let rel_path = file.strip_prefix(root).unwrap_or(file);
    let rel_str = rel_path.to_string_lossy().to_string();
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Lang::from_ext(ext);

    let role = extract_role(text, lang);
    let public_api = extract_public_api(text, &rel_str, lang);

    Ok(SmartSummary {
        file: rel_str,
        role,
        public_api,
    })
}

/// Smart summary using pre-computed FileData.
pub fn smart_summary_cached(rel_path: &str, filedata: &FileData) -> SmartSummary {
    SmartSummary {
        file: rel_path.to_string(),
        role: filedata.role.clone(),
        public_api: filedata.public_api.clone(),
    }
}

/// L1: Extract the role/description of the file.
/// Priority: first doc comment > first meaningful non-import line > filename
fn extract_role(text: &str, lang: Lang) -> String {
    // Compile symbol regex once, outside the per-line loop.
    let sym_patterns = lang.patterns();
    let sym_regex = if !sym_patterns.is_empty() {
        regex::Regex::new(sym_patterns).ok()
    } else {
        None
    };

    // Try to find a doc comment at the top
    for line in text.lines().take(20) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Doc comments
        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            let doc = trimmed
                .trim_start_matches("///")
                .trim_start_matches("//!")
                .trim();
            if !doc.is_empty() {
                return truncate(doc, 80);
            }
        }
        if trimmed.starts_with("/**") || trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
        {
            let doc = trimmed
                .trim_start_matches("/**")
                .trim_start_matches("\"\"\"")
                .trim_start_matches("'''")
                .trim_end_matches("*/")
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim();
            if !doc.is_empty() {
                return truncate(doc, 80);
            }
        }
        // Python/shell comments at top of file
        if trimmed.starts_with('#') && !trimmed.starts_with("#!") && !trimmed.starts_with("#[") {
            let doc = trimmed.trim_start_matches('#').trim();
            if !doc.is_empty() {
                return truncate(doc, 80);
            }
        }

        // Skip imports, use statements, package declarations
        if is_preamble_line(trimmed) {
            continue;
        }

        // First meaningful code line — use it as role hint
        if let Some(ref re) = sym_regex
            && re.is_match(line)
        {
            return truncate(trimmed.trim_end_matches('{').trim(), 80);
        }

        // Generic fallback: first non-empty, non-import line
        return truncate(trimmed, 80);
    }

    "empty file".to_string()
}

/// L2: Extract public API symbols.
fn extract_public_api(text: &str, file: &str, lang: Lang) -> String {
    let mut symbols = Vec::new();
    symbols::extract_from_text_pub(file, text, lang, &mut symbols);

    if symbols.is_empty() {
        return String::new();
    }

    // Extract just the names from signatures
    let names: Vec<String> = symbols
        .iter()
        .filter_map(|s| extract_symbol_name(&s.signature))
        .take(8) // Max 8 symbols
        .collect();

    names.join(", ")
}

/// Extract the symbol name from a signature line.
fn extract_symbol_name(sig: &str) -> Option<String> {
    let trimmed = sig.trim();

    // Try to find name after keywords
    let keywords = [
        "pub async fn ",
        "pub fn ",
        "pub(crate) fn ",
        "async fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "impl ",
        "pub type ",
        "type ",
        "pub mod ",
        "mod ",
        "pub const ",
        "const ",
        "pub static ",
        "static ",
        "export default function ",
        "export async function ",
        "export function ",
        "function ",
        "export default class ",
        "export class ",
        "class ",
        "export interface ",
        "interface ",
        "export type ",
        "export enum ",
        "export const ",
        "async def ",
        "def ",
        "func ",
    ];

    for kw in &keywords {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            // Take until ( or < or { or : or space or end
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }

    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Find a valid char boundary near max - 3
    let end = max.saturating_sub(3);
    let boundary = s.floor_char_boundary(end);
    format!("{}...", &s[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_extract_role_from_doc_comment() {
        let text = "/// Main entry point for the application\nfn main() {}\n";
        let role = extract_role(text, Lang::Rust);
        assert_eq!(role, "Main entry point for the application");
    }

    #[test]
    fn test_extract_role_from_python_docstring() {
        let text = "\"\"\"User authentication module\"\"\"\nimport os\n";
        let role = extract_role(text, Lang::Python);
        assert_eq!(role, "User authentication module");
    }

    #[test]
    fn test_extract_role_skips_imports() {
        let text =
            "use std::io;\nuse std::path::Path;\n\n/// Config parser\npub struct Config {}\n";
        let role = extract_role(text, Lang::Rust);
        assert_eq!(role, "Config parser");
    }

    #[test]
    fn test_extract_symbol_name() {
        assert_eq!(extract_symbol_name("pub fn main()"), Some("main".into()));
        assert_eq!(
            extract_symbol_name("export class UserService {"),
            Some("UserService".into())
        );
        assert_eq!(
            extract_symbol_name("def greet(name):"),
            Some("greet".into())
        );
        assert_eq!(
            extract_symbol_name("struct Config {"),
            Some("Config".into())
        );
    }

    #[test]
    fn test_smart_summarize_empty_dir() {
        let dir = TempDir::new().unwrap();
        let result = smart_summarize(dir.path(), true, 1_048_576, None, None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_smart_summarize_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "/// Math utilities").unwrap();
        writeln!(f, "pub fn add(a: i32, b: i32) -> i32 {{").unwrap();
        writeln!(f, "    a + b").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "pub fn multiply(a: i32, b: i32) -> i32 {{").unwrap();
        writeln!(f, "    a * b").unwrap();
        writeln!(f, "}}").unwrap();

        let result = smart_summarize_file(&file_path, dir.path()).unwrap();
        assert_eq!(result.role, "Math utilities");
        assert!(
            result.public_api.contains("add"),
            "got: {}",
            result.public_api
        );
        assert!(
            result.public_api.contains("multiply"),
            "got: {}",
            result.public_api
        );
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 80), "short");
        let long = "a".repeat(100);
        let result = truncate(&long, 20);
        assert_eq!(result.len(), 20);
        assert!(result.ends_with("..."));
    }
}
