mod cli;
mod daemon;
mod index;
mod output;
mod query;
mod search;
mod util;
mod walk;
mod watch;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{Cli, Commands};
use index::metadata::INDEX_VERSION;
use index::writer;
use output::printer::Printer;
use search::indexed;
use search::matcher::SearchConfig;
use util::{find_root, ig_dir};
use walk::DEFAULT_MAX_FILE_SIZE;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Search {
            pattern,
            path,
            after_context,
            before_context,
            context,
            ignore_case,
            count,
            files_with_matches,
            no_index,
            stats,
            file_type,
            glob,
            json,
            no_default_excludes,
            max_file_size,
        } => {
            let root = resolve_root(path.as_deref());
            let (before, after) = match context {
                Some(c) => (c, c),
                None => (before_context, after_context),
            };
            let config = SearchConfig {
                before_context: before,
                after_context: after,
                count_only: count,
                files_only: files_with_matches,
            };
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;

            let use_color = !json
                && atty::is(atty::Stream::Stdout)
                && std::env::var("NO_COLOR").is_err();

            if no_index {
                let results = search::fallback::search_brute_force(
                    &root,
                    &pattern,
                    ignore_case,
                    &config,
                    file_type.as_deref(),
                    glob.as_deref(),
                )?;
                let mut printer = Printer::new(use_color, json);
                for file_matches in &results {
                    printer.print_file_matches(file_matches, count, files_with_matches);
                }
            } else {
                // Auto-build or rebuild index if needed
                ensure_index(&root, use_excludes, max_size)?;

                let (results, search_stats) = indexed::search_indexed(
                    &root,
                    &pattern,
                    ignore_case,
                    &config,
                    file_type.as_deref(),
                    glob.as_deref(),
                )?;
                let mut printer = Printer::new(use_color, json);
                for file_matches in &results {
                    printer.print_file_matches(file_matches, count, files_with_matches);
                }
                if stats {
                    printer.print_stats(&search_stats);
                }
            }
        }

        Commands::Index {
            path,
            no_default_excludes,
            max_file_size,
        } => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;
            let start = Instant::now();
            let meta =
                writer::build_index(&root, use_excludes, max_size).context("building index")?;
            let elapsed = start.elapsed();
            let ig = ig_dir(&root);
            let size = dir_size(&ig);
            eprintln!(
                "Indexed {} files, {} trigrams in {:.1}s ({:.1} MB)",
                meta.file_count,
                meta.ngram_count,
                elapsed.as_secs_f64(),
                size as f64 / 1_048_576.0,
            );
            if let Some(ref commit) = meta.git_commit {
                eprintln!("Git commit: {}", &commit[..7.min(commit.len())]);
            }
        }

        Commands::Status { path } => {
            let root = resolve_root(path.as_deref());
            let ig = ig_dir(&root);
            if !index::metadata::IndexMetadata::exists(&ig) {
                eprintln!("No index found at {}", ig.display());
                eprintln!("Run `ig index` to build one.");
                std::process::exit(1);
            }
            let meta = index::metadata::IndexMetadata::load_from(&ig)?;
            let size = dir_size(&ig);
            let age_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .saturating_sub(meta.created_at);

            if meta.version != INDEX_VERSION {
                eprintln!(
                    "WARNING: Index version mismatch (have v{}, need v{}). Run `ig index` to rebuild.",
                    meta.version, INDEX_VERSION
                );
            }

            eprintln!(
                "Index: {} files, {} trigrams, {:.1} MB, built {}",
                meta.file_count,
                meta.ngram_count,
                size as f64 / 1_048_576.0,
                format_age(age_secs),
            );
            if let Some(ref commit) = meta.git_commit {
                eprintln!("Git commit: {}", &commit[..7.min(commit.len())]);
            }

            // Check daemon status
            let sock = daemon::socket_path(&root);
            if sock.exists() {
                eprintln!("Daemon: running ({})", sock.display());
            } else {
                eprintln!("Daemon: not running");
            }
        }

        Commands::Watch {
            path,
            no_default_excludes,
        } => {
            let root = resolve_root(path.as_deref());
            watch::watch_and_rebuild(&root, !no_default_excludes)?;
        }

        Commands::Daemon { path } => {
            let root = resolve_root(path.as_deref());
            // Ensure index exists
            ensure_index(&root, true, DEFAULT_MAX_FILE_SIZE)?;
            daemon::start_daemon(&root)?;
        }

        Commands::Query {
            pattern,
            path,
            ignore_case,
        } => {
            let root = resolve_root(path.as_deref());
            let response = daemon::query_daemon(&root, &pattern, ignore_case)?;
            print!("{}", response);
        }
    }

    Ok(())
}

/// Ensure the index exists and is up to date.
fn ensure_index(root: &std::path::Path, use_excludes: bool, max_size: u64) -> Result<()> {
    let ig = ig_dir(root);
    let needs_build = if !index::metadata::IndexMetadata::exists(&ig) {
        true
    } else {
        match index::metadata::IndexMetadata::load_from(&ig) {
            Ok(meta) => meta.version != INDEX_VERSION,
            Err(_) => true,
        }
    };

    if needs_build {
        eprintln!("Building index for {}...", root.display());
        let meta = writer::build_index(root, use_excludes, max_size).context("building index")?;
        eprintln!(
            "Indexed {} files, {} trigrams",
            meta.file_count, meta.ngram_count
        );
    }
    Ok(())
}

fn resolve_root(path: Option<&str>) -> PathBuf {
    let base = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    find_root(&base)
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
