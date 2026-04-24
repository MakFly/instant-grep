use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::context::BlockResult;
use crate::read::ReadResult;
use crate::search::indexed::SearchStats;
use crate::search::matcher::{FileMatches, LineMatch};
use crate::smart::{DirAggregate, SmartSummary};
use crate::symbols::SymbolMatch;

pub struct Printer {
    stdout: StandardStream,
    first_file: bool,
    json_mode: bool,
    max_line_len: usize,
    max_matches_per_file: usize,
    max_matches_total: usize,
    compact: bool,
    current_file_matches: usize,
    current_file_truncated_notice: bool,
    total_matches_emitted: usize,
    global_cap_hit: bool,
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
        let (max_line_len, max_matches_per_file, max_matches_total, compact) = compact_limits();
        Self {
            stdout: StandardStream::stdout(choice),
            first_file: true,
            json_mode,
            max_line_len,
            max_matches_per_file,
            max_matches_total,
            compact,
            current_file_matches: 0,
            current_file_truncated_notice: false,
            total_matches_emitted: 0,
            global_cap_hit: false,
        }
    }

    /// Emit a minimal "no matches" notice (compact mode only, non-JSON).
    /// Useful for agents so they distinguish "no results" from "tool crashed".
    pub fn print_no_matches(&mut self, pattern: &str) {
        if self.json_mode || !self.compact {
            return;
        }
        let _ = writeln!(self.stdout, "0 matches for {:?}", pattern);
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

        if self.global_cap_hit {
            return;
        }

        // Compact mode: no blank line between files (save 1 byte per file).
        if !self.first_file && !self.compact {
            let _ = writeln!(self.stdout);
        }
        self.first_file = false;

        self.print_file_path(&file_matches.path);
        let _ = writeln!(self.stdout);

        let mut prev_line_num: Option<usize> = None;
        self.current_file_matches = 0;
        self.current_file_truncated_notice = false;

        for line_match in &file_matches.matches {
            if self.max_matches_total > 0 && self.total_matches_emitted >= self.max_matches_total {
                let _ = self
                    .stdout
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)).set_dimmed(true));
                let _ = writeln!(
                    self.stdout,
                    "… global cap reached ({} matches)",
                    self.max_matches_total
                );
                let _ = self.stdout.reset();
                self.global_cap_hit = true;
                return;
            }

            if self.max_matches_per_file > 0
                && self.current_file_matches >= self.max_matches_per_file
            {
                if !self.current_file_truncated_notice {
                    let remaining = file_matches
                        .matches
                        .len()
                        .saturating_sub(self.current_file_matches);
                    let _ = self
                        .stdout
                        .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)).set_dimmed(true));
                    let _ = writeln!(self.stdout, "… +{} more", remaining);
                    let _ = self.stdout.reset();
                    self.current_file_truncated_notice = true;
                }
                break;
            }

            // Compact mode: skip `--` separators between non-contiguous matches.
            if !self.compact
                && let Some(prev) = prev_line_num
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
            self.current_file_matches += 1;
            self.total_matches_emitted += 1;
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
        if self.compact && path.len() > 60 {
            let _ = write!(self.stdout, "{}", ellide_path(path));
        } else {
            let _ = write!(self.stdout, "{}", path);
        }
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

        let owned_trunc: Vec<u8>;
        let mut ranges_adj: Vec<std::ops::Range<usize>> = line_match.match_ranges.clone();
        let line: &[u8] = if self.max_line_len > 0 && line_match.line.len() > self.max_line_len {
            owned_trunc = truncate_match_line(
                &line_match.line,
                &line_match.match_ranges,
                self.max_line_len,
                &mut ranges_adj,
            );
            &owned_trunc
        } else {
            &line_match.line
        };
        if ranges_adj.is_empty() || line_match.is_context {
            let _ = self.stdout.write_all(line);
        } else {
            let mut pos = 0;
            for range in &ranges_adj {
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
        // Compact mode (pipe) + large listing: emit a one-line aggregate instead
        // of every path. `--verbose` or TTY keeps the full list.
        if self.compact && !self.json_mode && files.len() >= 40 {
            self.print_file_list_aggregate(files, root);
            return;
        }
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

    fn print_file_list_aggregate(
        &mut self,
        files: &[std::path::PathBuf],
        root: &std::path::Path,
    ) {
        use std::collections::BTreeSet;

        let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut dirs: BTreeSet<std::path::PathBuf> = BTreeSet::new();

        for path in files {
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
        }

        // Top extensions by count, descending
        let mut ext_sorted: Vec<(String, usize)> = ext_counts.into_iter().collect();
        ext_sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let top_exts: Vec<String> = ext_sorted
            .iter()
            .take(6)
            .map(|(e, c)| format!("{} {}", c, e))
            .collect();

        let _ = writeln!(
            self.stdout,
            "{} files in {} dirs · {}",
            files.len(),
            dirs.len(),
            top_exts.join(", ")
        );
        let _ = writeln!(
            self.stdout,
            "(compact view — set IG_COMPACT=0 or run in a TTY for the full listing)"
        );
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

    pub fn print_read_plain(&mut self, result: &ReadResult) {
        for (_num, line) in &result.lines {
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
        ext_sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
        let ext_display: Vec<String> = ext_sorted
            .iter()
            .take(5)
            .map(|(ext, count)| format!("{} {}", ext, count))
            .collect();

        let _ = writeln!(self.stdout);
        let _ = self.stdout.set_color(ColorSpec::new().set_dimmed(true));
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
            let dir_name = dir.file_name().unwrap_or_default().to_string_lossy();

            let _ = self
                .stdout
                .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)).set_bold(true));
            let _ = write!(self.stdout, "{}{}/", prefix, dir_name);
            let _ = self.stdout.reset();
            let _ = self.stdout.set_color(ColorSpec::new().set_dimmed(true));
            let _ = writeln!(self.stdout, " ({})", total);
            let _ = self.stdout.reset();
        }

        let child_indent = if print_header { indent + 1 } else { indent };
        let child_prefix = "  ".repeat(child_indent);

        // Print files in this directory (excluding root which is printed separately)
        if print_header && let Some(filenames) = tree.get(dir) {
            for chunk in filenames.chunks(4) {
                let _ = writeln!(self.stdout, "{}{}", child_prefix, chunk.join("  "));
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
        let _ = write!(
            self.stdout,
            "}},\"summary\":{{\"files\":{},\"dirs\":{},\"extensions\":{{",
            total_files, total_dirs
        );
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

    /// One-block aggregate summary for a directory (compact pipe mode).
    pub fn print_dir_aggregate(&mut self, agg: &DirAggregate, dir_name: &str) {
        if self.json_mode {
            let key_files_json: Vec<String> = agg
                .key_files
                .iter()
                .map(|f| format!("\"{}\"", escape_json(f)))
                .collect();
            let exts_json: Vec<String> = agg
                .top_exts
                .iter()
                .map(|(e, c)| format!("\"{}\":{}", escape_json(e), c))
                .collect();
            let subdirs_json: Vec<String> = agg
                .top_subdirs
                .iter()
                .map(|(d, c)| format!("\"{}\":{}", escape_json(d), c))
                .collect();
            let _ = writeln!(
                self.stdout,
                "{{\"dir\":\"{}\",\"total_files\":{},\"dir_count\":{},\"top_exts\":{{{}}},\"top_subdirs\":{{{}}},\"key_files\":[{}]}}",
                escape_json(dir_name),
                agg.total_files,
                agg.dir_count,
                exts_json.join(","),
                subdirs_json.join(","),
                key_files_json.join(",")
            );
            return;
        }
        let exts: Vec<String> = agg
            .top_exts
            .iter()
            .map(|(e, c)| format!("{} {}", c, e))
            .collect();
        let _ = writeln!(
            self.stdout,
            "{}: {} files, {} dirs · {}",
            dir_name,
            agg.total_files,
            agg.dir_count,
            exts.join(", ")
        );
        if !agg.top_subdirs.is_empty() {
            let subs: Vec<String> = agg
                .top_subdirs
                .iter()
                .map(|(d, c)| format!("{}/ ({})", d, c))
                .collect();
            let _ = writeln!(self.stdout, "top: {}", subs.join(", "));
        }
        if !agg.key_files.is_empty() {
            let _ = writeln!(self.stdout, "key: {}", agg.key_files.join(", "));
        }
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
        .filter(|k| !k.as_os_str().is_empty() && *k != dir && k.starts_with(dir))
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

/// Shorten a path for compact display: keep the first segment and the last two.
/// `apps/pwa-backoffice/src/app/(auth)/maintenance/components/maintenance-client.tsx`
/// becomes
/// `apps/.../components/maintenance-client.tsx`.
/// If the result is still > 80 bytes, additionally truncate the filename.
fn ellide_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 3 {
        return path.to_string();
    }
    let first = parts[0];
    let tail = &parts[parts.len() - 2..];
    let mut out = format!("{}/.../{}", first, tail.join("/"));
    if out.len() > 80 {
        // Still too long (huge filename). Truncate the basename keeping extension.
        if let Some(base) = tail.last() {
            let (stem, ext) = match base.rsplit_once('.') {
                Some((s, e)) => (s, format!(".{}", e)),
                None => (*base, String::new()),
            };
            let keep = 80usize.saturating_sub(
                first.len() + 5 /* "/.../" */ + tail[0].len() + 1 /* "/" */ + ext.len() + 1, /* "…" */
            );
            let short_stem: String = stem.chars().take(keep).collect();
            out = format!(
                "{}/.../{}/{}…{}",
                first,
                tail[0],
                short_stem,
                ext
            );
        }
    }
    out
}

/// Read compact-mode env vars. Compact mode activates when any of:
/// - `IG_COMPACT=1` is set explicitly
/// - stdout is not a TTY (piped to a file / agent / `wc`) and `IG_COMPACT` is unset or `"auto"`
///
/// Defaults in compact mode (aligned with rtk 0.37 so agents see the same budget):
/// - line length capped at 80
/// - per-file match cap: 10 (ig-only — rtk has no per-file cap)
/// - global match cap: 200
/// - no `--` separator between non-contiguous matches
/// - no blank line between files
///
/// Opt-out: `IG_COMPACT=0` forces full verbose output even on pipe.
/// Fine-tune: `IG_LINE_MAX`, `IG_MAX_MATCHES_PER_FILE`, `IG_MAX_MATCHES_TOTAL` override caps.
fn compact_limits() -> (usize, usize, usize, bool) {
    let raw = std::env::var("IG_COMPACT").ok();
    let compact = match raw.as_deref() {
        Some("1") | Some("true") | Some("yes") => true,
        Some("0") | Some("false") | Some("no") => false,
        // unset or "auto" → enable on pipe
        _ => !std::io::stdout().is_terminal(),
    };
    let line_max = std::env::var("IG_LINE_MAX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(if compact { 80 } else { 0 });
    let per_file = std::env::var("IG_MAX_MATCHES_PER_FILE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(if compact { 10 } else { 0 });
    let total = std::env::var("IG_MAX_MATCHES_TOTAL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(if compact { 200 } else { 0 });
    (line_max, per_file, total, compact)
}

/// Truncate `line` to at most `max_len` bytes while preserving match markers.
/// Strategy: if a match fits within the window, keep the window around the first match.
/// Otherwise return `line[..first60] + "…" + line[len-30..]`.
/// Adjusts provided match ranges into the new buffer (or empties them if they fall outside).
fn truncate_match_line(
    line: &[u8],
    ranges: &[std::ops::Range<usize>],
    max_len: usize,
    out_ranges: &mut Vec<std::ops::Range<usize>>,
) -> Vec<u8> {
    out_ranges.clear();
    if line.len() <= max_len {
        out_ranges.extend_from_slice(ranges);
        return line.to_vec();
    }
    // Strip leading whitespace first — cheapest win.
    let lead = line
        .iter()
        .take_while(|b| **b == b' ' || **b == b'\t')
        .count();
    let trimmed = &line[lead..];
    if trimmed.len() <= max_len {
        for r in ranges {
            let s = r.start.saturating_sub(lead);
            let e = r.end.saturating_sub(lead);
            if e > s && s < trimmed.len() {
                out_ranges.push(s..e.min(trimmed.len()));
            }
        }
        return trimmed.to_vec();
    }

    // Build: first_head bytes + "…" + last_tail bytes (head+tail = max_len - 1 for ellipsis)
    let ellipsis: &[u8] = "…".as_bytes(); // 3 bytes
    let budget = max_len.saturating_sub(ellipsis.len());
    let head_len = (budget * 2 / 3).min(trimmed.len());
    let tail_len = budget
        .saturating_sub(head_len)
        .min(trimmed.len() - head_len);

    let head = &trimmed[..safe_utf8_boundary(trimmed, head_len, false)];
    let tail_start = trimmed.len() - tail_len;
    let tail = &trimmed[safe_utf8_boundary(trimmed, tail_start, true)..];

    let mut out = Vec::with_capacity(head.len() + ellipsis.len() + tail.len());
    out.extend_from_slice(head);
    out.extend_from_slice(ellipsis);
    out.extend_from_slice(tail);

    // Adjust ranges: keep those fully inside head; drop those that cross the cut.
    for r in ranges {
        let s = r.start.saturating_sub(lead);
        let e = r.end.saturating_sub(lead);
        if e <= head.len() {
            out_ranges.push(s..e);
        } else if s >= trimmed.len() - tail.len() {
            let off = head.len() + ellipsis.len();
            let ns = off + (s - (trimmed.len() - tail.len()));
            let ne = off + (e - (trimmed.len() - tail.len()));
            out_ranges.push(ns..ne);
        }
        // else: spans the cut, drop.
    }
    out
}

fn safe_utf8_boundary(buf: &[u8], mut idx: usize, forward: bool) -> usize {
    idx = idx.min(buf.len());
    if forward {
        while idx < buf.len() && (buf[idx] & 0xC0) == 0x80 {
            idx += 1;
        }
    } else {
        while idx > 0 && (buf[idx] & 0xC0) == 0x80 {
            idx -= 1;
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_files_recursive() {
        let mut tree: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        tree.insert(
            PathBuf::from(""),
            vec!["Cargo.toml".into(), "README.md".into()],
        );
        tree.insert(
            PathBuf::from("src"),
            vec!["main.rs".into(), "cli.rs".into()],
        );
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
        let files = vec![root.join("Cargo.toml"), root.join("src/main.rs")];

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
