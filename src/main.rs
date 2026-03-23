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
use index::metadata::{INDEX_VERSION, IndexMetadata};
use index::writer;
use output::printer::Printer;
use search::indexed;
use search::matcher::SearchConfig;
use util::{find_root, ig_dir};
use walk::DEFAULT_MAX_FILE_SIZE;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Extract search flags from top-level (shared between shortcut and subcommand)
    let ignore_case = cli.ignore_case;
    let after_context = cli.after_context;
    let before_context = cli.before_context;
    let context = cli.context;
    let count = cli.count;
    let files_with_matches = cli.files_with_matches;
    let no_index = cli.no_index;
    let stats = cli.stats;
    let file_type = cli.file_type;
    let glob = cli.glob;
    let json = cli.json;
    let no_default_excludes = cli.no_default_excludes;
    let max_file_size = cli.max_file_size;

    match cli.command {
        // Explicit subcommands
        Some(Commands::Search { pattern, path }) => {
            do_search(&SearchOpts {
                pattern: &pattern,
                path: path.as_deref(),
                ignore_case,
                after_context,
                before_context,
                context,
                count,
                files_with_matches,
                no_index,
                stats,
                file_type: file_type.as_deref(),
                glob: glob.as_deref(),
                json,
                no_default_excludes,
                max_file_size,
            })?;
        }

        Some(Commands::Index { path }) => {
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

        Some(Commands::Status { path }) => {
            let root = resolve_root(path.as_deref());
            let ig = ig_dir(&root);
            if !IndexMetadata::exists(&ig) {
                eprintln!("No index found at {}", ig.display());
                eprintln!("Run `ig index` to build one.");
                std::process::exit(1);
            }
            let meta = IndexMetadata::load_from(&ig)?;
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

            let sock = daemon::socket_path(&root);
            if sock.exists() {
                eprintln!("Daemon: running ({})", sock.display());
            } else {
                eprintln!("Daemon: not running");
            }
        }

        Some(Commands::Watch { path }) => {
            let root = resolve_root(path.as_deref());
            watch::watch_and_rebuild(&root, !no_default_excludes)?;
        }

        Some(Commands::Daemon { path }) => {
            let root = resolve_root(path.as_deref());
            ensure_index(&root, true, DEFAULT_MAX_FILE_SIZE)?;
            daemon::start_daemon(&root)?;
        }

        Some(Commands::Query { pattern, path }) => {
            let root = resolve_root(path.as_deref());
            let response = daemon::query_daemon(&root, &pattern, ignore_case)?;
            print!("{}", response);
        }

        // No subcommand — shortcut mode: `ig "pattern" [path]`
        None => {
            if let Some(pattern) = cli.pattern {
                do_search(&SearchOpts {
                    pattern: &pattern,
                    path: cli.path.as_deref(),
                    ignore_case,
                    after_context,
                    before_context,
                    context,
                    count,
                    files_with_matches,
                    no_index,
                    stats,
                    file_type: file_type.as_deref(),
                    glob: glob.as_deref(),
                    json,
                    no_default_excludes,
                    max_file_size,
                })?;
            } else {
                // No pattern, no subcommand — show help
                use clap::CommandFactory;
                Cli::command().print_help()?;
                println!();
            }
        }
    }

    Ok(())
}

struct SearchOpts<'a> {
    pattern: &'a str,
    path: Option<&'a str>,
    ignore_case: bool,
    after_context: usize,
    before_context: usize,
    context: Option<usize>,
    count: bool,
    files_with_matches: bool,
    no_index: bool,
    stats: bool,
    file_type: Option<&'a str>,
    glob: Option<&'a str>,
    json: bool,
    no_default_excludes: bool,
    max_file_size: Option<u64>,
}

/// Core search logic shared between `ig "pattern"` and `ig search "pattern"`.
#[allow(clippy::too_many_arguments)]
fn do_search(opts: &SearchOpts) -> Result<()> {
    let root = resolve_root(opts.path);
    let (before, after) = match opts.context {
        Some(c) => (c, c),
        None => (opts.before_context, opts.after_context),
    };
    let config = SearchConfig {
        before_context: before,
        after_context: after,
        count_only: opts.count,
        files_only: opts.files_with_matches,
    };
    let max_size = opts.max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
    let use_excludes = !opts.no_default_excludes;

    let use_color =
        !opts.json && atty::is(atty::Stream::Stdout) && std::env::var("NO_COLOR").is_err();

    if opts.no_index {
        let results = search::fallback::search_brute_force(
            &root,
            opts.pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
        )?;
        let mut printer = Printer::new(use_color, opts.json);
        for file_matches in &results {
            printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
        }
        return Ok(());
    }

    let ig = ig_dir(&root);
    let index_ready = IndexMetadata::exists(&ig)
        && IndexMetadata::load_from(&ig)
            .map(|m| m.version == INDEX_VERSION)
            .unwrap_or(false);

    if !index_ready {
        let results = search::fallback::search_brute_force(
            &root,
            opts.pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
        )?;
        let mut printer = Printer::new(use_color, opts.json);
        for file_matches in &results {
            printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
        }

        // Build index after results are printed (user sees output immediately)
        let root_clone = root.clone();
        let handle = std::thread::spawn(move || {
            let _ = writer::build_index(&root_clone, use_excludes, max_size);
        });
        let _ = handle.join();

        return Ok(());
    }

    let (results, search_stats) = indexed::search_indexed(
        &root,
        opts.pattern,
        opts.ignore_case,
        &config,
        opts.file_type,
        opts.glob,
    )?;
    let mut printer = Printer::new(use_color, opts.json);
    for file_matches in &results {
        printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
    }
    if opts.stats {
        printer.print_stats(&search_stats);
    }

    Ok(())
}

fn ensure_index(root: &std::path::Path, use_excludes: bool, max_size: u64) -> Result<()> {
    let ig = ig_dir(root);
    let needs_build = if !IndexMetadata::exists(&ig) {
        true
    } else {
        match IndexMetadata::load_from(&ig) {
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
