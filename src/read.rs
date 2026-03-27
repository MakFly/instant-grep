use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::symbols::Lang;
use crate::util::{is_binary, is_preamble_line};

pub struct ReadResult {
    pub file: String,
    pub lines: Vec<(usize, String)>,
}

/// Read a file and return numbered lines.
/// In signatures mode, only return import lines and symbol definitions.
pub fn read_file(file: &Path, signatures_only: bool) -> Result<ReadResult> {
    let content = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;

    if is_binary(&content) {
        bail!("binary file: {}", file.display());
    }

    let text = std::str::from_utf8(&content)
        .with_context(|| format!("{} is not valid UTF-8", file.display()))?;

    let file_str = file.to_string_lossy().to_string();

    if !signatures_only {
        let lines: Vec<(usize, String)> = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.to_string()))
            .collect();
        return Ok(ReadResult {
            file: file_str,
            lines,
        });
    }

    // Signatures mode: imports + symbol definitions
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
}
