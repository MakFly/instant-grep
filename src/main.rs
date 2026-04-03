mod brain;
mod cli;
mod context;
mod daemon;
mod discover;
mod gain;
mod git;
mod index;
mod ls;
mod output;
mod pack;
mod query;
mod read;
mod rewrite;
mod search;
mod setup;
mod smart;
mod symbols;
mod tracking;
mod uninstall;
mod update;
mod util;
mod walk;
mod watch;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{Cli, Commands};
use index::metadata::{INDEX_VERSION, IndexMetadata};
use index::overlay::OverlayReader;
use index::writer;
use output::printer::Printer;
use search::indexed;
use search::matcher::SearchConfig;
use util::{find_root, ig_dir};
use walk::DEFAULT_MAX_FILE_SIZE;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Check for updates in the background (non-blocking)
    update::check_update_background();

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
    let word_regexp = cli.word_regexp;
    let fixed_strings = cli.fixed_strings;
    let no_default_excludes = cli.no_default_excludes;
    let max_file_size = cli.max_file_size;

    match cli.command {
        // Explicit subcommands
        Some(Commands::Search { pattern, paths }) => {
            do_search(&SearchOpts {
                pattern: &pattern,
                paths: &paths,
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
                word_regexp,
                fixed_strings,
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

            // Account for overlay files (added/modified since last full rebuild)
            let total_file_count = if let Ok(Some(overlay)) = OverlayReader::open(&ig) {
                meta.file_count + overlay.metadata.overlay_file_count
            } else {
                meta.file_count
            };
            eprintln!(
                "Index: {} files, {} trigrams, {:.1} MB, built {}",
                total_file_count,
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

        Some(Commands::Daemon { action, path }) => {
            let root = resolve_root(path.as_deref());
            match action.as_deref() {
                Some("stop") => daemon::stop_daemon(&root)?,
                Some("status") => daemon::daemon_status(&root)?,
                Some("install") => {
                    ensure_index(&root, true, DEFAULT_MAX_FILE_SIZE)?;
                    daemon::install_launchd(&root)?;
                }
                Some("uninstall") => daemon::uninstall_launchd(&root)?,
                Some("start") => {
                    ensure_index(&root, true, DEFAULT_MAX_FILE_SIZE)?;
                    daemon::start_daemon_background(&root)?;
                }
                None | Some("foreground") => {
                    // Default: foreground mode (backward compat, also used by launchd)
                    ensure_index(&root, true, DEFAULT_MAX_FILE_SIZE)?;
                    daemon::start_daemon(&root)?;
                }
                Some(other) => {
                    eprintln!("Unknown daemon action: {}", other);
                    eprintln!("Available: start, stop, status, install, uninstall");
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Files { path }) => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;
            let files = walk::walk_files(
                &root,
                use_excludes,
                max_size,
                file_type.as_deref(),
                glob.as_deref(),
            )?;
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_file_list(&files, &root);
        }

        Some(Commands::Symbols { path }) => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;
            let syms = symbols::extract_symbols(
                &root,
                use_excludes,
                max_size,
                file_type.as_deref(),
                glob.as_deref(),
            )?;
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_symbols(&syms);
        }

        Some(Commands::Context { file, line }) => {
            let path = std::path::Path::new(&file);
            let block = context::extract_block(path, line)?;
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_context(&block);
        }

        Some(Commands::Read { file, signatures }) => {
            let path = std::path::Path::new(&file);
            let original_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let result = read::read_file(path, signatures)?;
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_read(&result);

            // Track savings (cat would output original_size bytes)
            let output_bytes: u64 = result.lines.iter().map(|(_, l)| l.len() as u64 + 7).sum(); // +7 for line num prefix
            tracking::log_savings(&tracking::TrackEntry {
                command: if signatures {
                    format!("ig read -s {}", file)
                } else {
                    format!("ig read {}", file)
                },
                original_bytes: original_size,
                output_bytes,
                project: std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
            });
        }

        Some(Commands::Smart { path }) => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;

            let base_path = path.as_deref().map(std::path::Path::new);
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);

            if let Some(p) = base_path
                && p.is_file()
            {
                // Single file
                let s = smart::smart_summarize_file(p, &root)?;
                printer.print_smart(&[s]);
            } else {
                // Directory — use the specified path if it's a subdir, otherwise root
                let scan_dir = if let Some(p) = base_path
                    && p.is_dir()
                {
                    p.canonicalize().unwrap_or_else(|_| root.clone())
                } else {
                    root.clone()
                };
                let summaries = smart::smart_summarize(
                    &scan_dir,
                    use_excludes,
                    max_size,
                    file_type.as_deref(),
                    glob.as_deref(),
                )?;
                printer.print_smart(&summaries);
            }
        }

        Some(Commands::Pack { path }) => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;
            let start = Instant::now();

            // Ensure index exists first (tree.txt is generated during indexing)
            ensure_index(&root, use_excludes, max_size)?;

            let output = pack::generate_context(&root, use_excludes, max_size)?;
            let elapsed = start.elapsed();
            let lines = output.lines().count();
            let ig = ig_dir(&root);
            eprintln!(
                "Generated .ig/context.md: {} lines in {:.1}s",
                lines,
                elapsed.as_secs_f64(),
            );
            eprintln!("Path: {}", ig.join("context.md").display());
        }

        Some(Commands::Ls { path }) => {
            let target = path.as_deref().unwrap_or(".");
            let result = ls::compact_ls(std::path::Path::new(target))?;
            let output = ls::format_ls(&result);
            print!("{}", output);

            // Track savings: estimate ls -la output as ~80 bytes/entry
            let estimated_original = (result.total_files + result.total_dirs) as u64 * 80;
            let output_bytes = output.len() as u64;
            tracking::log_savings(&tracking::TrackEntry {
                command: format!("ig ls {}", target),
                original_bytes: estimated_original,
                output_bytes,
                project: std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
            });
        }

        Some(Commands::Git { args }) => {
            git::run_git(&args);
        }

        Some(Commands::Rewrite { command }) => {
            rewrite::run_rewrite(&command);
        }

        Some(Commands::Gain {
            clear,
            history,
            json: gain_json,
        }) => {
            gain::show_gain(clear, history, gain_json);
        }

        Some(Commands::Proxy { command: proxy_cmd }) => {
            if proxy_cmd.is_empty() {
                eprintln!("Usage: ig proxy <command...>");
                std::process::exit(1);
            }
            let status = std::process::Command::new(&proxy_cmd[0])
                .args(&proxy_cmd[1..])
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to execute {}: {}", proxy_cmd[0], e);
                    std::process::exit(1);
                });
            std::process::exit(status.code().unwrap_or(1));
        }

        Some(Commands::Discover { since, limit }) => {
            discover::run_discover(since, limit);
        }

        Some(Commands::Completions { shell }) => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(shell, &mut cmd, "ig", &mut std::io::stdout());
        }

        Some(Commands::Setup { dry_run }) => {
            setup::run_setup(dry_run);
        }

        Some(Commands::Uninstall { dry_run, yes }) => {
            uninstall::run_uninstall(dry_run, yes);
        }

        Some(Commands::Brain { action }) => match action.as_str() {
            "login" => brain::brain_login()?,
            "sync" => brain::brain_sync()?,
            "pull" => brain::brain_pull()?,
            "status" => brain::brain_status()?,
            _ => {
                eprintln!("Unknown brain action: {action}");
                eprintln!("Usage: ig brain [login|sync|pull|status]");
                std::process::exit(1);
            }
        },

        Some(Commands::Update) => {
            update::run_update()?;
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
                    paths: &cli.paths,
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
                    word_regexp,
                    fixed_strings,
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
    paths: &'a [String],
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
    word_regexp: bool,
    fixed_strings: bool,
    no_default_excludes: bool,
    max_file_size: Option<u64>,
}

/// Transform pattern based on -w and -F flags.
fn prepare_pattern(pattern: &str, word_regexp: bool, fixed_strings: bool) -> String {
    let mut p = if fixed_strings {
        regex::escape(pattern)
    } else {
        pattern.to_string()
    };
    if word_regexp {
        p = format!(r"\b{}\b", p);
    }
    p
}

/// Core search logic shared between `ig "pattern"` and `ig search "pattern"`.
#[allow(clippy::too_many_arguments)]
fn do_search(opts: &SearchOpts) -> Result<()> {
    // Reject empty patterns — they match everything and waste tokens
    if opts.pattern.is_empty() {
        anyhow::bail!("empty pattern — provide a search term");
    }

    // Resolve paths: detect single-file, directory prefixes, or default to cwd
    let (root, path_filters) = resolve_root_and_filters(opts.paths);
    let pattern = prepare_pattern(opts.pattern, opts.word_regexp, opts.fixed_strings);
    let pattern = pattern.as_str();
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

    let use_color = util::use_color(opts.json);

    if opts.no_index {
        let results = search::fallback::search_brute_force(
            &root,
            pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
            &path_filters,
            max_size,
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
            pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
            &path_filters,
            max_size,
        )?;
        let mut printer = Printer::new(use_color, opts.json);
        for file_matches in &results {
            printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
        }

        // Build index after results are printed (user sees output immediately)
        let _ = writer::build_index(&root, use_excludes, max_size);

        return Ok(());
    }

    let (results, search_stats) = indexed::search_indexed(
        &root,
        pattern,
        opts.ignore_case,
        &config,
        opts.file_type,
        opts.glob,
        &path_filters,
        max_size,
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

/// Resolve multiple paths into (project_root, path_filters).
/// Each filter is a relative path (file or directory prefix) under the root.
/// Empty input means no filtering (search everything).
fn resolve_root_and_filters(paths: &[String]) -> (PathBuf, Vec<String>) {
    if paths.is_empty() {
        return (
            find_root(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            Vec::new(),
        );
    }

    // Single path: use it to find the root (backward compat)
    // Multiple paths: use cwd to find the root, all paths are relative filters
    let root = if paths.len() == 1 {
        find_root(&PathBuf::from(&paths[0]))
    } else {
        find_root(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    };

    let root_abs = root.canonicalize().unwrap_or_else(|_| root.clone());
    let mut filters = Vec::new();

    for p in paths {
        let base = PathBuf::from(p);
        if let Ok(abs) = base.canonicalize()
            && let Ok(rel) = abs.strip_prefix(&root_abs)
        {
            let mut rel_str = rel.to_string_lossy().to_string();
            // For directories, ensure trailing slash so starts_with works as prefix
            if base.is_dir() && !rel_str.is_empty() && !rel_str.ends_with('/') {
                rel_str.push('/');
            }
            filters.push(rel_str);
        }
    }

    (root, filters)
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
