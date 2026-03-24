use std::path::Path;

use anyhow::Result;
use regex::Regex;

use crate::util::is_binary;
use crate::walk;

pub struct SymbolMatch {
    pub file: String,
    pub line: usize,
    pub kind: &'static str,
    pub signature: String,
}

pub fn extract_symbols(
    root: &Path,
    use_default_excludes: bool,
    max_file_size: u64,
    type_filter: Option<&str>,
    glob_filter: Option<&str>,
) -> Result<Vec<SymbolMatch>> {
    let files = walk::walk_files(root, use_default_excludes, max_file_size, type_filter, glob_filter)?;
    let mut symbols = Vec::new();

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
        let rel_str = rel_path.to_string_lossy();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Lang::from_ext(ext);

        extract_from_text(&rel_str, text, lang, &mut symbols);
    }

    Ok(symbols)
}

fn extract_from_text(file: &str, text: &str, lang: Lang, symbols: &mut Vec<SymbolMatch>) {
    let patterns = lang.patterns();
    if patterns.is_empty() {
        return;
    }

    let regex = match Regex::new(patterns) {
        Ok(r) => r,
        Err(_) => return,
    };

    for (line_num, line) in text.lines().enumerate() {
        if let Some(m) = regex.find(line) {
            let signature = line.trim().to_string();
            // Truncate very long signatures
            let signature = if signature.len() > 120 {
                format!("{}...", &signature[..117])
            } else {
                signature
            };

            let kind = classify_kind(m.as_str(), lang);

            symbols.push(SymbolMatch {
                file: file.to_string(),
                line: line_num + 1,
                kind,
                signature,
            });
        }
    }
}

fn classify_kind(matched: &str, lang: Lang) -> &'static str {
    // Use the raw match (with trailing space from regex) for keyword detection
    let m = matched;
    match lang {
        Lang::Rust => {
            if m.contains("fn ") { "function" }
            else if m.contains("struct ") { "struct" }
            else if m.contains("enum ") { "enum" }
            else if m.contains("trait ") { "trait" }
            else if m.contains("impl ") { "impl" }
            else if m.contains("type ") { "type" }
            else if m.contains("mod ") { "module" }
            else if m.contains("const ") { "const" }
            else if m.contains("static ") { "static" }
            else { "symbol" }
        }
        Lang::TypeScript | Lang::JavaScript => {
            if m.contains("function") || m.contains("=>") { "function" }
            else if m.contains("class ") { "class" }
            else if m.contains("interface ") { "interface" }
            else if m.contains("type ") { "type" }
            else if m.contains("enum ") { "enum" }
            else if m.contains("const ") { "const" }
            else { "symbol" }
        }
        Lang::Python => {
            if m.contains("def ") { "function" }
            else if m.contains("class ") { "class" }
            else { "symbol" }
        }
        Lang::Go => {
            if m.contains("func ") { "function" }
            else if m.contains("type ") { "type" }
            else { "symbol" }
        }
        Lang::Php => {
            if m.contains("function ") { "function" }
            else if m.contains("class ") { "class" }
            else if m.contains("interface ") { "interface" }
            else if m.contains("trait ") { "trait" }
            else if m.contains("enum ") { "enum" }
            else { "symbol" }
        }
        Lang::Other => {
            if m.contains("fn ") || m.contains("func ") || m.contains("function ") || m.contains("def ") { "function" }
            else if m.contains("class ") { "class" }
            else if m.contains("struct ") { "struct" }
            else if m.contains("interface ") { "interface" }
            else if m.contains("type ") { "type" }
            else { "symbol" }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Lang {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Php,
    Other,
}

impl Lang {
    fn from_ext(ext: &str) -> Self {
        match ext {
            "rs" => Lang::Rust,
            "ts" | "tsx" => Lang::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "py" | "pyi" => Lang::Python,
            "go" => Lang::Go,
            "php" => Lang::Php,
            "vue" | "svelte" => Lang::TypeScript, // script blocks use TS patterns
            _ => Lang::Other,
        }
    }

    fn patterns(&self) -> &'static str {
        match self {
            Lang::Rust => r"^\s*(pub(\(crate\))?\s+)?(async\s+)?(fn|struct|enum|trait|impl|type|mod|const|static)\s+",
            Lang::TypeScript | Lang::JavaScript => r"^\s*(export\s+)?(default\s+)?(async\s+)?(function\*?\s+|class\s+|interface\s+|type\s+|enum\s+|const\s+\w+\s*=\s*(async\s+)?\()",
            Lang::Python => r"^\s*(async\s+)?(def|class)\s+",
            Lang::Go => r"^(func|type)\s+",
            Lang::Php => r"^\s*(abstract\s+|final\s+)*(public|protected|private|static)?\s*(function|class|interface|trait|enum)\s+",
            Lang::Other => r"^\s*(export\s+)?(pub\s+)?(async\s+)?(function\*?\s+|class\s+|def\s+|fn\s+|struct\s+|type\s+|interface\s+|trait\s+|impl\s+)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper: extract symbols using extract_from_text directly, bypassing the
    // filesystem walker. This tests lang detection and pattern matching without
    // relying on `ignore`'s type-filter or git-ignore rules which can differ
    // across environments.
    fn rust_syms_from_text() -> Vec<SymbolMatch> {
        let mut out = Vec::new();
        let text = "pub fn main() {\n    println!(\"hello\");\n}\n\nstruct Config {\n    name: String,\n}\n\npub enum Status {\n    Active,\n    Inactive,\n}\n";
        extract_from_text("src/main.rs", text, Lang::Rust, &mut out);
        out
    }

    #[test]
    fn test_rust_symbols() {
        let syms = rust_syms_from_text();
        assert!(!syms.is_empty(), "should find at least one Rust symbol");
        let sigs: Vec<&str> = syms.iter().map(|s| s.signature.as_str()).collect();
        assert!(
            sigs.iter().any(|s| s.contains("fn main")),
            "should find fn main, got: {:?}",
            sigs
        );
        assert!(
            sigs.iter().any(|s| s.contains("struct Config")),
            "should find struct Config"
        );
        assert!(
            sigs.iter().any(|s| s.contains("enum Status")),
            "should find enum Status"
        );
    }

    #[test]
    fn test_rust_symbol_kinds() {
        // classify_kind receives the *matched* substring (before trim removes
        // trailing whitespace). Verify that bare `fn`, `struct`, and `enum`
        // keywords (without pub prefix) are classified correctly.
        let mut out = Vec::new();
        let text = "fn bare_fn() {}\nstruct Bare;\nenum BareEnum {}\n";
        extract_from_text("src/bare.rs", text, Lang::Rust, &mut out);
        let kinds: Vec<&str> = out.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&"function"), "bare fn should be 'function'");
        assert!(kinds.contains(&"struct"), "bare struct should be 'struct'");
        assert!(kinds.contains(&"enum"), "bare enum should be 'enum'");
    }

    #[test]
    fn test_typescript_symbols() {
        let mut syms = Vec::new();
        let text = "export function hello() {\n  return 'hi';\n}\n\nexport class AppService {\n  run() {}\n}\n\ninterface Config {\n  name: string;\n}\n";
        extract_from_text("src/app.ts", text, Lang::TypeScript, &mut syms);
        assert!(!syms.is_empty(), "should find TypeScript symbols");
        let has_function = syms.iter().any(|s| s.kind == "function");
        let has_class = syms.iter().any(|s| s.kind == "class");
        assert!(has_function, "should find export function");
        assert!(has_class, "should find export class");
    }

    #[test]
    fn test_python_symbols() {
        let mut syms = Vec::new();
        let text = "def greet(name):\n    return f'hello {name}'\n\nclass UserService:\n    def __init__(self):\n        pass\n\nasync def fetch_data():\n    pass\n";
        extract_from_text("src/lib.py", text, Lang::Python, &mut syms);
        assert!(!syms.is_empty(), "should find Python symbols");
        let has_def = syms.iter().any(|s| s.kind == "function");
        let has_class = syms.iter().any(|s| s.kind == "class");
        assert!(has_def, "should find def");
        assert!(has_class, "should find class");
    }

    #[test]
    fn test_symbols_empty_dir() {
        let dir = TempDir::new().unwrap();
        let syms = extract_symbols(dir.path(), true, 1_048_576, None, None).unwrap();
        assert!(syms.is_empty());
    }
}
