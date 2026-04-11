use std::path::PathBuf;

use regex::Regex;
use serde::Deserialize;

use super::pipeline::CompiledFilter;

/// Will be populated by build.rs concatenating all filters/*.toml
const BUILTIN_FILTERS: &str = include_str!(concat!(env!("OUT_DIR"), "/builtin_filters.toml"));

#[derive(Deserialize)]
struct FilterFile {
    #[serde(default)]
    filters: Vec<FilterDef>,
}

#[derive(Deserialize)]
struct FilterDef {
    name: String,
    #[serde(rename = "match")]
    match_pattern: String,
    #[serde(default)]
    strip_ansi: bool,
    #[serde(default)]
    replace: Vec<ReplaceDef>,
    keep_lines: Option<String>,
    drop_lines: Option<String>,
    truncate_at: Option<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    max_lines: Option<usize>,
    on_empty: Option<String>,
}

#[derive(Deserialize)]
struct ReplaceDef {
    find: String,
    with: String,
}

/// Load and compile all filters from builtin, user, and project sources.
pub fn load_filters() -> Vec<CompiledFilter> {
    let mut filters = Vec::new();

    // 1. Builtin filters
    if !BUILTIN_FILTERS.is_empty() {
        if let Some(mut compiled) = parse_and_compile(BUILTIN_FILTERS, "builtin") {
            filters.append(&mut compiled);
        }
    }

    // 2. User filters from ~/.config/ig/filters/*.toml
    if let Some(user_dir) = user_filters_dir() {
        load_from_dir(&user_dir, &mut filters, "user");
    }

    // 3. Project filters from .ig/filters/*.toml (trust-gated)
    let project_dir = PathBuf::from(".ig/filters");
    if project_dir.is_dir() {
        load_from_dir_trusted(&project_dir, &mut filters);
    }

    filters
}

/// Get the user filters directory: ~/.config/ig/filters/
fn user_filters_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ig").join("filters"))
}

/// Load all .toml files from a directory and compile them.
fn load_from_dir(dir: &PathBuf, filters: &mut Vec<CompiledFilter>, source: &str) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let label = format!("{}:{}", source, path.display());
                    if let Some(mut compiled) = parse_and_compile(&content, &label) {
                        filters.append(&mut compiled);
                    }
                }
                Err(e) => {
                    eprintln!("ig: warn: failed to read {}: {}", path.display(), e);
                }
            }
        }
    }
}

/// Load project-local .toml filter files with trust verification.
fn load_from_dir_trusted(dir: &PathBuf, filters: &mut Vec<CompiledFilter>) {
    use crate::trust;

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            if !matches!(trust::check_trust(&path), trust::TrustStatus::Trusted) {
                eprintln!("ig: warn: untrusted filter skipped: {}", path.display());
                eprintln!("  Run `ig trust {}` to trust it", path.display());
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let label = format!("project:{}", path.display());
                    if let Some(mut compiled) = parse_and_compile(&content, &label) {
                        filters.append(&mut compiled);
                    }
                }
                Err(e) => {
                    eprintln!("ig: warn: failed to read {}: {}", path.display(), e);
                }
            }
        }
    }
}

/// Parse TOML content and compile into CompiledFilters.
fn parse_and_compile(content: &str, source: &str) -> Option<Vec<CompiledFilter>> {
    let file: FilterFile = match toml::from_str(content) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ig: warn: failed to parse filters from {}: {}", source, e);
            return None;
        }
    };

    let mut compiled = Vec::new();
    for def in file.filters {
        match compile_filter(def) {
            Ok(f) => compiled.push(f),
            Err(e) => {
                eprintln!("ig: warn: skipping filter from {}: {}", source, e);
            }
        }
    }

    Some(compiled)
}

/// Compile a FilterDef into a CompiledFilter by compiling all regex patterns.
fn compile_filter(def: FilterDef) -> Result<CompiledFilter, String> {
    let match_regex = Regex::new(&def.match_pattern)
        .map_err(|e| format!("bad match regex '{}': {}", def.match_pattern, e))?;

    let mut replace_rules = Vec::new();
    for r in &def.replace {
        let re =
            Regex::new(&r.find).map_err(|e| format!("bad replace regex '{}': {}", r.find, e))?;
        replace_rules.push((re, r.with.clone()));
    }

    let keep_lines = def
        .keep_lines
        .as_deref()
        .map(Regex::new)
        .transpose()
        .map_err(|e| format!("bad keep_lines regex: {}", e))?;

    let drop_lines = def
        .drop_lines
        .as_deref()
        .map(Regex::new)
        .transpose()
        .map_err(|e| format!("bad drop_lines regex: {}", e))?;

    if keep_lines.is_some() && drop_lines.is_some() {
        return Err(format!(
            "filter '{}': keep_lines and drop_lines are mutually exclusive",
            def.name
        ));
    }

    Ok(CompiledFilter {
        name: def.name,
        match_regex,
        strip_ansi: def.strip_ansi,
        replace_rules,
        keep_lines,
        drop_lines,
        truncate_at: def.truncate_at,
        head: def.head,
        tail: def.tail,
        max_lines: def.max_lines,
        on_empty: def.on_empty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_compile_valid() {
        let toml = r#"
[[filters]]
name = "cargo-test"
match = "^cargo test"
strip_ansi = true
replace = [
  { find = "^\\s+Compiling .+$", with = "" },
]
keep_lines = "^(test |error)"
head = 50
on_empty = "All tests passed"
"#;
        let result = parse_and_compile(toml, "test");
        assert!(result.is_some());
        let filters = result.unwrap();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].name, "cargo-test");
        assert!(filters[0].strip_ansi);
        assert_eq!(filters[0].replace_rules.len(), 1);
        assert!(filters[0].keep_lines.is_some());
        assert!(filters[0].drop_lines.is_none());
        assert_eq!(filters[0].head, Some(50));
        assert_eq!(filters[0].on_empty.as_deref(), Some("All tests passed"));
    }

    #[test]
    fn test_parse_and_compile_invalid_toml() {
        let result = parse_and_compile("not valid toml {{", "test");
        assert!(result.is_none());
    }

    #[test]
    fn test_compile_filter_bad_regex() {
        let def = FilterDef {
            name: "bad".to_string(),
            match_pattern: "[invalid".to_string(),
            strip_ansi: false,
            replace: vec![],
            keep_lines: None,
            drop_lines: None,
            truncate_at: None,
            head: None,
            tail: None,
            max_lines: None,
            on_empty: None,
        };
        assert!(compile_filter(def).is_err());
    }

    #[test]
    fn test_mutually_exclusive_keep_drop() {
        let def = FilterDef {
            name: "both".to_string(),
            match_pattern: ".*".to_string(),
            strip_ansi: false,
            replace: vec![],
            keep_lines: Some("foo".to_string()),
            drop_lines: Some("bar".to_string()),
            truncate_at: None,
            head: None,
            tail: None,
            max_lines: None,
            on_empty: None,
        };
        let err = compile_filter(def).unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn test_load_filters_returns_vec() {
        // Should not panic even with no filter files present
        let filters = load_filters();
        // Just verify it returns without error
        let _ = filters;
    }

    #[test]
    fn test_multiple_filters_in_one_file() {
        let toml = r#"
[[filters]]
name = "first"
match = "^first"

[[filters]]
name = "second"
match = "^second"
drop_lines = "^debug"
"#;
        let result = parse_and_compile(toml, "test").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "first");
        assert_eq!(result[1].name, "second");
        assert!(result[1].drop_lines.is_some());
    }
}
