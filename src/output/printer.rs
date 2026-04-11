use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::context::BlockResult;
use crate::read::ReadResult;
use crate::search::indexed::SearchStats;
use crate::search::matcher::{FileMatches, LineMatch};
use crate::smart::SmartSummary;
use crate::symbols::SymbolMatch;

pub struct Printer {
    stdout: StandardStream,
    first_file: bool,
    json_mode: bool,
}

impl Printer {
    pub fn new(color: bool, json_mode: bool) -> Self {
        let choice = if json_mode {
            ColorChoice::Never
        } else if color {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        };
        Self {
            stdout: StandardStream::stdout(choice),
            first_file: true,
            json_mode,
        }
    }

    pub fn print_file_matches(
        &mut self,
        file_matches: &FileMatches,
        count_only: bool,
        files_only: bool,
    ) {
        if self.json_mode {
            self.print_json(file_matches, count_only, files_only);
            return;
        }

        if files_only {
            self.print_file_path(&file_matches.path);
            let _ = writeln!(self.stdout);
            return;
        }

        if count_only {
            self.print_file_path(&file_matches.path);
            let _ = write!(self.stdout, ":");
            let _ = writeln!(self.stdout, "{}", file_matches.match_count);
            return;
        }

        if !self.first_file {
            let _ = writeln!(self.stdout);
        }
        self.first_file = false;

        self.print_file_path(&file_matches.path);
        let _ = writeln!(self.stdout);

        let mut prev_line_num: Option<usize> = None;

        for line_match in &file_matches.matches {
            if let Some(prev) = prev_line_num
                && line_match.line_number > prev + 1
            {
                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
                let _ = writeln!(self.stdout, "--");
                let _ = self.stdout.reset();
            }
            prev_line_num = Some(line_match.line_number);

            self.print_line(line_match);
        }
    }

    fn print_json(&mut self, file_matches: &FileMatches, count_only: bool, files_only: bool) {
        if files_only {
            let _ = writeln!(
                self.stdout,
                "{{\"file\":\"{}\"}}",
                escape_json(&file_matches.path)
            );
            return;
        }

        if count_only {
            let _ = writeln!(
                self.stdout,
                "{{\"file\":\"{}\",\"count\":{}}}",
                escape_json(&file_matches.path),
                file_matches.match_count
            );
            return;
        }

        for line_match in &file_matches.matches {
            if line_match.is_context {
                continue;
            }
            let line_text = String::from_utf8_lossy(&line_match.line);
            let _ = writeln!(
                self.stdout,
                "{{\"file\":\"{}\",\"line\":{},\"text\":\"{}\"}}",
                escape_json(&file_matches.path),
                line_match.line_number,
                escape_json(&line_text),
            );
        }
    }

    fn print_file_path(&mut self, path: &str) {
        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Magenta)).set_bold(true));
        let _ = write!(self.stdout, "{}", path);
        let _ = self.stdout.reset();
    }

    fn print_line(&mut self, line_match: &LineMatch) {
        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Green)));
        let _ = write!(self.stdout, "{}", line_match.line_number);
        let _ = self.stdout.reset();

        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
        if line_match.is_context {
            let _ = write!(self.stdout, "-");
        } else {
            let _ = write!(self.stdout, ":");
        }
        let _ = self.stdout.reset();

        let line = &line_match.line;
        if line_match.match_ranges.is_empty() || line_match.is_context {
            let _ = self.stdout.write_all(line);
        } else {
            let mut pos = 0;
            for range in &line_match.match_ranges {
                let start = range.start.min(line.len());
                let end = range.end.min(line.len());

                if pos < start {
                    let _ = self.stdout.write_all(&line[pos..start]);
                }

                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true));
                let _ = self.stdout.write_all(&line[start..end]);
                let _ = self.stdout.reset();

                pos = end;
            }

            if pos < line.len() {
                let _ = self.stdout.write_all(&line[pos..]);
            }
        }

        let _ = writeln!(self.stdout);
    }

    pub fn print_stats(&mut self, stats: &SearchStats) {
        if self.json_mode {
            let _ = writeln!(
                self.stdout,
                "{{\"_stats\":{{\"candidates\":{},\"total\":{},\"search_ms\":{:.1},\"used_index\":{}}}}}",
                stats.candidate_files,
                stats.total_files,
                stats.search_duration.as_secs_f64() * 1000.0,
                stats.used_index,
            );
            return;
        }

        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Yellow)));
        let pct = if stats.total_files > 0 {
            (stats.candidate_files as f64 / stats.total_files as f64) * 100.0
        } else {
            0.0
        };
        let _ = writeln!(
            self.stdout,
            "\n--- stats ---\nCandidates: {}/{} files ({:.1}%)\nSearch: {}\nIndex: {}",
            stats.candidate_files,
            stats.total_files,
            pct,
            format_duration(stats.search_duration),
            if stats.used_index {
                "yes"
            } else {
                "no (fallback)"
            },
        );
        let _ = self.stdout.reset();
    }

    pub fn print_file_list(&mut self, files: &[std::path::PathBuf], root: &std::path::Path) {
        for path in files {
            let rel = path.strip_prefix(root).unwrap_or(path);
            let rel_str = rel.to_string_lossy();
            if self.json_mode {
                let _ = writeln!(self.stdout, "{{\"file\":\"{}\"}}", escape_json(&rel_str));
            } else {
                let _ = writeln!(self.stdout, "{}", rel_str);
            }
        }
    }

    pub fn print_symbols(&mut self, symbols: &[SymbolMatch]) {
        for sym in symbols {
            if self.json_mode {
                let _ = writeln!(
                    self.stdout,
                    "{{\"file\":\"{}\",\"line\":{},\"kind\":\"{}\",\"symbol\":\"{}\"}}",
                    escape_json(&sym.file),
                    sym.line,
                    sym.kind,
                    escape_json(&sym.signature),
                );
            } else {
                self.print_file_path(&sym.file);
                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
                let _ = write!(self.stdout, ":");
                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Green)));
                let _ = write!(self.stdout, "{}", sym.line);
                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
                let _ = write!(self.stdout, ":");
                let _ = self.stdout.reset();
                let _ = writeln!(self.stdout, "{}", sym.signature);
            }
        }
    }

    pub fn print_context(&mut self, block: &BlockResult) {
        if self.json_mode {
            let lines_json: Vec<String> = block
                .lines
                .iter()
                .map(|(num, text)| {
                    format!("{{\"line\":{},\"text\":\"{}\"}}", num, escape_json(text))
                })
                .collect();
            let _ = writeln!(
                self.stdout,
                "{{\"file\":\"{}\",\"start\":{},\"end\":{},\"lines\":[{}]}}",
                escape_json(&block.file),
                block.start,
                block.end,
                lines_json.join(","),
            );
            return;
        }

        // Header
        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Magenta)).set_bold(true));
        let _ = write!(self.stdout, "{}", block.file);
        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
        let _ = writeln!(self.stdout, ":{}-{}", block.start, block.end);
        let _ = self.stdout.reset();

        // Lines
        let width = block.end.to_string().len();
        for (num, text) in &block.lines {
            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Green)));
            let _ = write!(self.stdout, "{:>width$}", num, width = width);
            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
            let _ = write!(self.stdout, " │ ");
            let _ = self.stdout.reset();
            let _ = writeln!(self.stdout, "{}", text);
        }
    }

    pub fn print_read(&mut self, result: &ReadResult) {
        if self.json_mode {
            for (num, line) in &result.lines {
                let _ = writeln!(
                    self.stdout,
                    "{{\"file\":\"{}\",\"line\":{},\"text\":\"{}\"}}",
                    escape_json(&result.file),
                    num,
                    escape_json(line)
                );
            }
            return;
        }

        for (num, line) in &result.lines {
            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Green)).set_dimmed(true));
            let _ = write!(self.stdout, "{:>4}: ", num);
            let _ = self.stdout.reset();
            let _ = writeln!(self.stdout, "{}", line);
        }
    }

    pub fn print_file_tree(&mut self, files: &[PathBuf], root: &Path) {
        // Convert to relative paths
        let rel_paths: Vec<PathBuf> = files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap_or(p).to_path_buf())
            .collect();

        // Build tree: dir -> sorted list of filenames
        let mut tree: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        for rel in &rel_paths {
            let parent = rel.parent().unwrap_or_else(|| Path::new(""));
            let filename = rel
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            tree.entry(parent.to_path_buf()).or_default().push(filename);
        }

        // Sort filenames within each directory
        for filenames in tree.values_mut() {
            filenames.sort();
        }

        // Collect extension stats
        let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
        for rel in &rel_paths {
            let ext = rel
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_else(|| "none".to_string());
            *ext_counts.entry(ext).or_insert(0) += 1;
        }

        // Count unique directories (excluding root "")
        let dir_count = tree.keys().filter(|k| !k.as_os_str().is_empty()).count();

        if self.json_mode {
            self.print_file_tree_json(&tree, &ext_counts, files.len(), dir_count);
            return;
        }

        // Recursive print from root
        let root_path = PathBuf::from("");
        self.print_tree_dir(&tree, &root_path, 0, false);

        // Root files (files with parent == "")
        if let Some(root_files) = tree.get(&root_path) {
            for chunk in root_files.chunks(4) {
                let _ = writeln!(self.stdout, "{}", chunk.join("  "));
            }
        }

        // Summary line
        let mut ext_sorted: Vec<_> = ext_counts.into_iter().collect();
        ext_sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let ext_display: Vec<String> = ext_sorted
            .iter()
            .take(5)
            .map(|(ext, count)| format!("{} {}", ext, count))
            .collect();

        let _ = writeln!(self.stdout);
        let _ = self
            .stdout
            .set_color(ColorSpec::new().set_dimmed(true));
        let _ = writeln!(
            self.stdout,
            "Summary: {} files, {} dirs ({})",
            files.len(),
            dir_count,
            ext_display.join(", "),
        );
        let _ = self.stdout.reset();
    }

    /// Recursively print a directory and its subdirectories.
    fn print_tree_dir(
        &mut self,
        tree: &BTreeMap<PathBuf, Vec<String>>,
        dir: &PathBuf,
        indent: usize,
        print_header: bool,
    ) {
        let prefix = "  ".repeat(indent);

        if print_header {
            // Count all files recursively under this dir
            let total = count_files_recursive(tree, dir);
            let dir_name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();

            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)).set_bold(true));
            let _ = write!(self.stdout, "{}{}/", prefix, dir_name);
            let _ = self.stdout.reset();
            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_dimmed(true));
            let _ = writeln!(self.stdout, " ({})", total);
            let _ = self.stdout.reset();
        }

        let child_indent = if print_header { indent + 1 } else { indent };
        let child_prefix = "  ".repeat(child_indent);

        // Print files in this directory (excluding root which is printed separately)
        if print_header {
            if let Some(filenames) = tree.get(dir) {
                for chunk in filenames.chunks(4) {
                    let _ = writeln!(self.stdout, "{}{}", child_prefix, chunk.join("  "));
                }
            }
        }

        // Find and print subdirectories
        let mut subdirs: Vec<&PathBuf> = tree
            .keys()
            .filter(|k| {
                if k.as_os_str().is_empty() || *k == dir {
                    return false;
                }
                k.parent() == Some(dir.as_path())
            })
            .collect();
        subdirs.sort();

        for subdir in subdirs {
            self.print_tree_dir(tree, subdir, child_indent, true);
        }
    }

    fn print_file_tree_json(
        &mut self,
        tree: &BTreeMap<PathBuf, Vec<String>>,
        ext_counts: &BTreeMap<String, usize>,
        total_files: usize,
        total_dirs: usize,
    ) {
        // Build JSON structure
        let _ = write!(self.stdout, "{{\"tree\":{{");
        let mut first_dir = true;
        for (dir, filenames) in tree {
            if !first_dir {
                let _ = write!(self.stdout, ",");
            }
            first_dir = false;
            let dir_str = if dir.as_os_str().is_empty() {
                ".".to_string()
            } else {
                dir.to_string_lossy().to_string()
            };
            let files_json: Vec<String> = filenames
                .iter()
                .map(|f| format!("\"{}\"", escape_json(f)))
                .collect();
            let _ = write!(
                self.stdout,
                "\"{}\":[{}]",
                escape_json(&dir_str),
                files_json.join(",")
            );
        }
        let _ = write!(self.stdout, "}},\"summary\":{{\"files\":{},\"dirs\":{},\"extensions\":{{", total_files, total_dirs);
        let mut first_ext = true;
        for (ext, count) in ext_counts {
            if !first_ext {
                let _ = write!(self.stdout, ",");
            }
            first_ext = false;
            let _ = write!(self.stdout, "\"{}\":{}", escape_json(ext), count);
        }
        let _ = writeln!(self.stdout, "}}}}}}");
    }

    pub fn print_smart(&mut self, summaries: &[SmartSummary]) {
        if self.json_mode {
            for s in summaries {
                let _ = writeln!(
                    self.stdout,
                    "{{\"file\":\"{}\",\"role\":\"{}\",\"public_api\":\"{}\"}}",
                    escape_json(&s.file),
                    escape_json(&s.role),
                    escape_json(&s.public_api)
                );
            }
            return;
        }

        for s in summaries {
            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
            let _ = write!(self.stdout, "{}", s.file);
            let _ = self.stdout.reset();
            let _ = write!(self.stdout, " — {}", s.role);
            if !s.public_api.is_empty() {
                let _ = self.stdout.set_color(ColorSpec::new().set_dimmed(true));
                let _ = write!(self.stdout, " / {}", s.public_api);
                let _ = self.stdout.reset();
            }
            let _ = writeln!(self.stdout);
        }
    }
}

/// Count all files recursively under a directory in the tree.
fn count_files_recursive(tree: &BTreeMap<PathBuf, Vec<String>>, dir: &PathBuf) -> usize {
    let own = tree.get(dir).map(|v| v.len()).unwrap_or(0);
    let children: usize = tree
        .keys()
        .filter(|k| {
            !k.as_os_str().is_empty() && *k != dir && k.starts_with(dir)
        })
        .map(|k| tree.get(k).map(|v| v.len()).unwrap_or(0))
        .sum();
    own + children
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.0}μs", d.as_micros())
    } else if ms < 1000.0 {
        format!("{:.1}ms", ms)
    } else {
        format!("{:.2}s", d.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_files_recursive() {
        let mut tree: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        tree.insert(PathBuf::from(""), vec!["Cargo.toml".into(), "README.md".into()]);
        tree.insert(PathBuf::from("src"), vec!["main.rs".into(), "cli.rs".into()]);
        tree.insert(
            PathBuf::from("src/index"),
            vec!["ngram.rs".into(), "reader.rs".into(), "writer.rs".into()],
        );

        // Root has 2 own files + 5 in subtrees = 7
        assert_eq!(count_files_recursive(&tree, &PathBuf::from("")), 7);
        // src/ has 2 own + 3 in src/index = 5
        assert_eq!(count_files_recursive(&tree, &PathBuf::from("src")), 5);
        // src/index has 3 own, no children
        assert_eq!(count_files_recursive(&tree, &PathBuf::from("src/index")), 3);
    }

    #[test]
    fn test_file_tree_output_format() {
        let root = PathBuf::from("/tmp/test_project");
        let files = vec![
            root.join("Cargo.toml"),
            root.join("README.md"),
            root.join("src/main.rs"),
            root.join("src/cli.rs"),
            root.join("src/index/ngram.rs"),
            root.join("src/index/reader.rs"),
        ];

        // Just verify it doesn't panic and produces output
        let mut printer = Printer::new(false, false);
        printer.print_file_tree(&files, &root);
    }

    #[test]
    fn test_file_tree_json_output() {
        let root = PathBuf::from("/tmp/test_project");
        let files = vec![
            root.join("Cargo.toml"),
            root.join("src/main.rs"),
        ];

        // JSON mode should not panic
        let mut printer = Printer::new(false, true);
        printer.print_file_tree(&files, &root);
    }

    #[test]
    fn test_file_tree_empty() {
        let root = PathBuf::from("/tmp/empty");
        let files: Vec<PathBuf> = vec![];

        let mut printer = Printer::new(false, false);
        printer.print_file_tree(&files, &root);
    }
}
