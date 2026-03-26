use std::io::Write;
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
                    format!(
                        "{{\"line\":{},\"text\":\"{}\"}}",
                        num,
                        escape_json(text)
                    )
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
            let _ = self.stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)).set_dimmed(true));
            let _ = write!(self.stdout, "{:>4}: ", num);
            let _ = self.stdout.reset();
            let _ = writeln!(self.stdout, "{}", line);
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
            let _ = self.stdout.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)));
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
