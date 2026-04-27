mod analytics;
mod autoignore;
mod cli;
mod cmds;
mod config;
mod context;
mod daemon;
mod delta;
mod discover;
mod embed_poc;
mod filter;
mod gain;
mod git;
mod hooks;
mod index;
mod ls;
mod output;
mod pack;
mod query;
mod read;
mod rewrite;
mod runner;
mod scoring;
mod search;
mod semantic;
mod setup;
mod smart;
mod symbols;
mod tee;
mod tracking;
mod trust;
mod uninstall;
mod update;
mod util;
mod verify;
mod walk;
mod watch;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{Cli, Commands, EmbedPocOp, TeeOp};
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
    let compact = cli.compact;
    let no_index = cli.no_index;
    let stats = cli.stats;
    let file_type = cli.file_type;
    let glob = cli.glob;
    let json = cli.json;
    let word_regexp = cli.word_regexp;
    let fixed_strings = cli.fixed_strings;
    let no_default_excludes = cli.no_default_excludes;
    let max_file_size = cli.max_file_size;
    let top = cli.top;
    let semantic = cli.semantic;

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
                compact,
                no_index,
                stats,
                file_type: file_type.as_deref(),
                glob: glob.as_deref(),
                json,
                word_regexp,
                fixed_strings,
                no_default_excludes,
                max_file_size,
                top,
                semantic,
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

        Some(Commands::Files {
            path,
            compact: files_compact,
        }) => {
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
            if files_compact || compact {
                printer.print_file_tree(&files, &root);
            } else {
                printer.print_file_list(&files, &root);
            }

            let command = match path.as_deref() {
                Some(target) => format!("ig files {}", target),
                None => "ig files".to_string(),
            };
            tracking::log_usage(command);
        }

        Some(Commands::Symbols { path }) => {
            let root = resolve_root(path.as_deref());
            let scope = path.as_deref().map(std::path::Path::new).and_then(|p| {
                // Resolve to absolute path for scope filtering
                if p.is_absolute() {
                    Some(p.to_path_buf())
                } else {
                    p.canonicalize().ok()
                }
            });
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;
            let syms = symbols::extract_symbols(
                &root,
                scope.as_deref(),
                use_excludes,
                max_size,
                file_type.as_deref(),
                glob.as_deref(),
            )?;
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_symbols(&syms);

            let command = match path.as_deref() {
                Some(target) => format!("ig symbols {}", target),
                None => "ig symbols".to_string(),
            };
            tracking::log_usage(command);
        }

        Some(Commands::Context { file, line }) => {
            let path = std::path::Path::new(&file);
            let block = {
                let root =
                    find_root(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                let ig = ig_dir(&root);
                let fdi = index::filedata::FileDataIndex::load(&ig);
                let rel = path.canonicalize().ok().and_then(|abs| {
                    abs.strip_prefix(root.canonicalize().unwrap_or(root.clone()))
                        .ok()
                        .map(|r| r.to_string_lossy().to_string())
                });
                if let Some(ref fdi) = fdi
                    && let Some(ref rel) = rel
                    && let Some(fd) = fdi.get(rel)
                {
                    context::extract_block_cached(path, line, fd)?
                } else {
                    context::extract_block(path, line)?
                }
            };
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);
            printer.print_context(&block);
            tracking::log_usage(format!("ig context {}:{}", file, line));
        }

        Some(Commands::Read {
            file,
            signatures,
            aggressive,
            budget,
            relevant,
            delta,
            plain,
        }) => {
            let path = std::path::Path::new(&file);
            let original_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

            // Delta mode: show only git-changed lines with enclosing context
            let delta_done = if delta {
                if let Ok(result) = delta::read_delta(path) {
                    let use_color = util::use_color(json);
                    let mut printer = Printer::new(use_color, json);
                    printer.print_read(&result);
                    let output_bytes: u64 =
                        result.lines.iter().map(|(_, l)| l.len() as u64 + 7).sum();
                    tracking::log_savings(&tracking::TrackEntry {
                        command: format!("ig read -d {}", file),
                        original_bytes: original_size,
                        output_bytes,
                        project: tracking::current_project(),
                    });
                    true
                } else {
                    false // No git changes — fall through to signatures-only mode
                }
            } else {
                false
            };

            if !delta_done {
                // --plain: raw cat-equivalent output, no line numbers, no colors
                if plain {
                    let result = read::read_file_filtered(path, read::FilterLevel::Full)?;
                    let mut printer = Printer::new(false, false);
                    printer.print_read_plain(&result);
                    tracking::log_usage(format!("ig read --plain {}", file));
                    return Ok(());
                }

                let use_lsc = budget.is_some() || aggressive;
                let level = if delta {
                    // Delta with no changes falls back to signatures
                    read::FilterLevel::Signatures
                } else if use_lsc {
                    read::FilterLevel::Aggressive
                } else if signatures {
                    read::FilterLevel::Signatures
                } else {
                    read::FilterLevel::Full
                };

                // Try cached signatures when in Signatures mode
                let mut result = if level == read::FilterLevel::Signatures {
                    let root =
                        find_root(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                    let ig = ig_dir(&root);
                    let fdi = index::filedata::FileDataIndex::load(&ig);
                    let rel = path.canonicalize().ok().and_then(|abs| {
                        abs.strip_prefix(root.canonicalize().unwrap_or(root.clone()))
                            .ok()
                            .map(|r| r.to_string_lossy().to_string())
                    });
                    if let Some(ref fdi) = fdi
                        && let Some(ref rel) = rel
                        && let Some(fd) = fdi.get(rel)
                    {
                        read::read_signatures_cached(path, fd)?
                    } else {
                        read::read_file_filtered(path, level)?
                    }
                } else {
                    read::read_file_filtered(path, level)?
                };

                // Apply Layered Semantic Compression (Phases 2-5) on aggressive output
                if use_lsc {
                    result.lines = scoring::compress_lsc(result.lines, budget, relevant.as_deref());
                }

                let use_color = util::use_color(json);
                let mut printer = Printer::new(use_color, json);
                printer.print_read(&result);

                // Track savings
                let output_bytes: u64 = result.lines.iter().map(|(_, l)| l.len() as u64 + 7).sum();
                let flag = if let Some(b) = budget {
                    format!(" -b {}", b)
                } else if aggressive {
                    " -a".to_string()
                } else if signatures {
                    " -s".to_string()
                } else {
                    String::new()
                };
                tracking::log_savings(&tracking::TrackEntry {
                    command: format!("ig read{} {}", flag, file),
                    original_bytes: original_size,
                    output_bytes,
                    project: tracking::current_project(),
                });
            } // if !delta_done
        }

        Some(Commands::Smart { path }) => {
            let root = resolve_root(path.as_deref());
            let max_size = max_file_size.unwrap_or(DEFAULT_MAX_FILE_SIZE);
            let use_excludes = !no_default_excludes;

            let base_path = path.as_deref().map(std::path::Path::new);
            let use_color = util::use_color(json);
            let mut printer = Printer::new(use_color, json);

            // Load filedata cache
            let ig = ig_dir(&root);
            let fdi = index::filedata::FileDataIndex::load(&ig);

            if let Some(p) = base_path
                && p.is_file()
            {
                // Single file -- try cached first
                let rel = p.canonicalize().ok().and_then(|abs| {
                    abs.strip_prefix(root.canonicalize().unwrap_or(root.clone()))
                        .ok()
                        .map(|r| r.to_string_lossy().to_string())
                });
                let s = if let Some(ref fdi) = fdi
                    && let Some(ref rel) = rel
                    && let Some(fd) = fdi.get(rel)
                {
                    smart::smart_summary_cached(rel, fd)
                } else {
                    smart::smart_summarize_file(p, &root)?
                };
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

                // Compact pipe mode → emit a fast dir aggregate instead of
                // reading every file (per-file smart on a big tree is 5+ s).
                let is_compact = std::env::var("IG_COMPACT").ok().as_deref() != Some("0")
                    && (std::env::var("IG_COMPACT").ok().as_deref() == Some("1")
                        || !std::io::IsTerminal::is_terminal(&std::io::stdout()));
                if is_compact && !json {
                    let agg = smart::smart_dir_aggregate(
                        &scan_dir,
                        use_excludes,
                        max_size,
                        file_type.as_deref(),
                        glob.as_deref(),
                    )?;
                    let label = base_path
                        .and_then(|p| p.to_str())
                        .unwrap_or_else(|| scan_dir.to_str().unwrap_or("."));
                    printer.print_dir_aggregate(&agg, label);
                } else {
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

            let command = match path.as_deref() {
                Some(target) => format!("ig smart {}", target),
                None => "ig smart".to_string(),
            };
            tracking::log_usage(command);
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
                project: tracking::current_project(),
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
            full,
            history,
            json: gain_json,
            project,
            graph,
            quota,
            tier,
            daily,
            weekly,
            monthly,
            discover: gain_discover,
            since,
            missed,
            compare,
        }) => {
            if gain_discover {
                discover::run_discover(since, 15);
            } else if missed {
                discover::run_shell_history_scan(since, 15);
            } else if let Some(spec) = compare {
                gain::show_compare(&spec, gain_json);
            } else {
                gain::show_gain(gain::GainOpts {
                    clear,
                    full,
                    history,
                    json: gain_json,
                    project,
                    graph,
                    quota,
                    tier,
                    daily,
                    weekly,
                    monthly,
                });
            }
        }

        Some(Commands::Raw { command: raw_cmd }) => {
            if raw_cmd.is_empty() {
                eprintln!("Usage: ig raw <command...>");
                std::process::exit(1);
            }
            let status = std::process::Command::new(&raw_cmd[0])
                .args(&raw_cmd[1..])
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to execute {}: {}", raw_cmd[0], e);
                    std::process::exit(1);
                });
            std::process::exit(status.code().unwrap_or(1));
        }

        Some(Commands::Discover {
            since,
            limit,
            shell,
        }) => {
            discover::run_discover(since, limit);
            if shell {
                discover::run_shell_history_scan(since, limit);
            }
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

        Some(Commands::Update) => {
            update::run_update()?;
        }

        Some(Commands::Query { pattern, path }) => {
            let root = resolve_root(path.as_deref());
            let response = daemon::query_daemon(&root, &pattern, ignore_case)?;
            print!("{}", response);
        }

        Some(Commands::Run { args }) => {
            let code = cmds::run::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Tee { op }) => match op {
            TeeOp::Show { id } => match tee::read(&id) {
                Some(bytes) => {
                    use std::io::Write;
                    std::io::stdout().write_all(&bytes)?;
                }
                None => {
                    eprintln!("tee entry not found: {}", id);
                    std::process::exit(1);
                }
            },
            TeeOp::List => {
                let entries = tee::list();
                if entries.is_empty() {
                    eprintln!("no tee entries");
                } else {
                    for e in entries {
                        let age = std::time::SystemTime::now()
                            .duration_since(e.modified)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        println!("{}  {:>8} B  {}s ago", e.id, e.bytes, age);
                    }
                }
            }
            TeeOp::Clear => {
                let n = tee::clear();
                eprintln!("removed {} tee entries", n);
            }
        },

        Some(Commands::Err { args }) => {
            let code = cmds::err::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Test { args }) => {
            let code = cmds::test_runner::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Json { file, schema }) => {
            let mut json_args = vec![file];
            if schema {
                json_args.push("--schema".to_string());
            }
            let code = cmds::json_cmd::run(&json_args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Deps) => {
            let code = cmds::deps::run(&[])?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Docker { args }) => {
            let code = cmds::docker::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Env { pattern }) => {
            let env_args: Vec<String> = pattern.into_iter().collect();
            let code = cmds::env_cmd::run(&env_args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Diff { file1, file2 }) => {
            let code = cmds::diff_cmd::run(&[file1, file2])?;
            if code != 0 {
                std::process::exit(code);
            }
        }

        Some(Commands::Trust { file, list }) => {
            if list {
                let entries = trust::list_trusted();
                if entries.is_empty() {
                    eprintln!("No trusted filter files.");
                } else {
                    for (path, hash) in &entries {
                        eprintln!("{} ({})", path, &hash[..8]);
                    }
                }
            } else if let Some(f) = file {
                trust::trust_path(std::path::Path::new(&f))?;
                eprintln!("Trusted: {}", f);
            } else {
                eprintln!("Usage: ig trust <file> or ig trust --list");
            }
        }

        Some(Commands::Untrust { file }) => {
            trust::untrust_path(std::path::Path::new(&file))?;
            eprintln!("Untrusted: {}", file);
        }

        Some(Commands::Verify) => {
            verify::run_verify();
        }

        Some(Commands::Learn { since, limit }) => {
            analytics::learn::run_learn(since, limit);
        }

        Some(Commands::Session { since }) => {
            analytics::session::run_session(since);
        }

        Some(Commands::Economics { since }) => {
            analytics::economics::run_economics(since);
        }

        Some(Commands::Autoignore { path, force }) => {
            autoignore::run_autoignore(path, force)?;
        }

        Some(Commands::EmbedPoc { op }) => match op {
            EmbedPocOp::Hello { text } => embed_poc::run_hello(&text)?,
            EmbedPocOp::Index { dir, yes } => embed_poc::run_index(dir, yes)?,
            EmbedPocOp::Inspect { limit } => embed_poc::run_inspect(limit)?,
            EmbedPocOp::Search { query, top } => embed_poc::run_search(&query, top)?,
        },

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
                    compact,
                    no_index,
                    stats,
                    file_type: file_type.as_deref(),
                    glob: glob.as_deref(),
                    json,
                    word_regexp,
                    fixed_strings,
                    no_default_excludes,
                    max_file_size,
                    top,
                    semantic,
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
    compact: bool,
    no_index: bool,
    stats: bool,
    file_type: Option<&'a str>,
    glob: Option<&'a str>,
    json: bool,
    word_regexp: bool,
    fixed_strings: bool,
    no_default_excludes: bool,
    max_file_size: Option<u64>,
    top: Option<usize>,
    semantic: bool,
}

/// Does `s` look like a plain identifier — no regex metacharacters?
/// Safe to splice into an alternation.
fn is_plain_word(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
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

/// Convert a daemon response into `FileMatches` so the existing Printer
/// can render results identically to the in-process search path.
/// Match ranges are recomputed locally with the pattern (cheap), so
/// highlighting is preserved.
fn daemon_response_to_file_matches(
    resp: &daemon::DaemonResponse,
    pattern: &str,
    case_insensitive: bool,
    count_only: bool,
    files_only: bool,
) -> Vec<search::matcher::FileMatches> {
    use search::matcher::{FileMatches, LineMatch};
    let Some(matches) = resp.results.as_ref() else {
        return Vec::new();
    };

    // Compile regex once for highlight range recomputation.
    let regex = (!files_only && !count_only)
        .then(|| {
            regex::bytes::RegexBuilder::new(pattern)
                .case_insensitive(case_insensitive)
                .unicode(false)
                .build()
                .ok()
        })
        .flatten();

    let mut grouped: std::collections::BTreeMap<String, FileMatches> =
        std::collections::BTreeMap::new();

    for m in matches {
        let entry = grouped
            .entry(m.file.clone())
            .or_insert_with(|| FileMatches {
                path: m.file.clone(),
                matches: Vec::new(),
                match_count: 0,
            });

        if files_only {
            entry.match_count = entry.match_count.max(1);
            continue;
        }
        if count_only {
            entry.match_count = m.count.unwrap_or(0);
            continue;
        }

        let text = m.text.clone().unwrap_or_default();
        let bytes = text.as_bytes().to_vec();
        let match_ranges = match &regex {
            Some(re) => re.find_iter(&bytes).map(|mtch| mtch.range()).collect(),
            None => Vec::new(),
        };
        entry.matches.push(LineMatch {
            line_number: m.line.unwrap_or(0),
            line: bytes,
            match_ranges,
            is_context: false,
        });
        entry.match_count += 1;
    }

    grouped.into_values().collect()
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

    // Base pattern from -w / -F flags.
    let base_pattern = prepare_pattern(opts.pattern, opts.word_regexp, opts.fixed_strings);

    // Optional semantic expansion: rewrite single-word literal queries as
    // `\b(pattern|neigh1|neigh2|…)\b` using learned PMI co-occurrences.
    // Only kicks in when the user asked for --semantic AND the pattern is a
    // plain identifier (no regex metacharacters) — otherwise we'd corrupt it.
    let expanded_pattern = if opts.semantic && is_plain_word(opts.pattern) {
        let ig = ig_dir(&root);
        match semantic::cooccur::CooccurrenceIndex::load(&ig) {
            Some(idx) => match idx.expand(opts.pattern, 6) {
                Some(neigh) if !neigh.is_empty() => {
                    let terms: Vec<String> = std::iter::once(opts.pattern.to_string())
                        .chain(neigh)
                        .map(|t| regex::escape(&t))
                        .collect();
                    eprintln!(
                        "(semantic: expanded '{}' → {})",
                        opts.pattern,
                        terms[1..].join(", ")
                    );
                    Some(format!(r"\b({})\b", terms.join("|")))
                }
                _ => None,
            },
            None => {
                eprintln!(
                    "(semantic: no cooccurrence index — run `ig index` first; falling back to literal search)"
                );
                None
            }
        }
    } else {
        None
    };
    let pattern = expanded_pattern.as_deref().unwrap_or(base_pattern.as_str());
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
        let mut results = search::fallback::search_brute_force(
            &root,
            pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
            &path_filters,
            max_size,
        )?;
        if let Some(n) = opts.top {
            search::rank::rank_top(&mut results, &root, n);
        }
        if opts.compact {
            print_compact(&results);
        } else {
            let mut printer = Printer::new(use_color, opts.json);
            if results.is_empty() && !opts.count && !opts.files_with_matches {
                printer.print_no_matches(pattern);
            }
            for file_matches in &results {
                printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
            }
        }
        tracking::log_usage(search_command_label(opts));
        return Ok(());
    }

    let ig = ig_dir(&root);
    let index_ready = IndexMetadata::exists(&ig)
        && IndexMetadata::load_from(&ig)
            .map(|m| m.version == INDEX_VERSION)
            .unwrap_or(false);

    if !index_ready {
        let mut results = search::fallback::search_brute_force(
            &root,
            pattern,
            opts.ignore_case,
            &config,
            opts.file_type,
            opts.glob,
            &path_filters,
            max_size,
        )?;
        if let Some(n) = opts.top {
            search::rank::rank_top(&mut results, &root, n);
        }
        if opts.compact {
            print_compact(&results);
        } else {
            let mut printer = Printer::new(use_color, opts.json);
            if results.is_empty() && !opts.count && !opts.files_with_matches {
                printer.print_no_matches(pattern);
            }
            for file_matches in &results {
                printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
            }
        }

        // Build index after results are printed (user sees output immediately)
        let _ = writer::build_index(&root, use_excludes, max_size);

        tracking::log_usage(search_command_label(opts));
        return Ok(());
    }

    // ── Daemon auto-route ─────────────────────────────────────────────
    // For the agent hot path (`ig "x" path` with no advanced flags) we
    // try to short-circuit through a running daemon. This skips binary
    // cold start + index mmap page faults entirely (typical: 30–100 ms
    // → < 5 ms). Falls back transparently to in-process search if the
    // daemon is missing or the request is not representable.
    let daemon_eligible = !opts.json
        && !opts.stats
        && opts.top.is_none()
        && opts.glob.is_none()
        && path_filters.is_empty()
        && before == after
        && std::env::var_os("IG_NO_DAEMON").is_none();

    if daemon_eligible
        && let Ok(Some(resp)) = daemon::try_query_daemon(
            &root,
            pattern,
            opts.ignore_case,
            opts.files_with_matches,
            opts.count,
            before,
            opts.file_type,
        )
    {
        let results = daemon_response_to_file_matches(
            &resp,
            pattern,
            opts.ignore_case,
            opts.count,
            opts.files_with_matches,
        );
        if opts.compact {
            print_compact(&results);
        } else {
            let mut printer = Printer::new(use_color, opts.json);
            if results.is_empty() && !opts.count && !opts.files_with_matches {
                printer.print_no_matches(pattern);
            }
            for file_matches in &results {
                printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
            }
        }
        tracking::log_usage(search_command_label(opts));
        return Ok(());
    }

    // Daemon was not reachable — best-effort auto-spawn for next call.
    if daemon_eligible
        && std::env::var_os("IG_NO_AUTO_DAEMON").is_none()
        && !daemon::is_daemon_available(&root)
    {
        let _ = daemon::start_daemon_background_silent(&root);
    }

    let (mut results, search_stats) = indexed::search_indexed(
        &root,
        pattern,
        opts.ignore_case,
        &config,
        opts.file_type,
        opts.glob,
        &path_filters,
        max_size,
    )?;

    if let Some(n) = opts.top {
        search::rank::rank_top(&mut results, &root, n);
    }

    if opts.compact {
        print_compact(&results);
    } else {
        let mut printer = Printer::new(use_color, opts.json);
        if results.is_empty() && !opts.count && !opts.files_with_matches {
            printer.print_no_matches(pattern);
        }
        for file_matches in &results {
            printer.print_file_matches(file_matches, opts.count, opts.files_with_matches);
        }
    }
    if opts.stats {
        let mut printer = Printer::new(use_color, opts.json);
        printer.print_stats(&search_stats);
    }

    tracking::log_usage(search_command_label(opts));

    Ok(())
}

/// Compact output for --compact mode: header + truncated matches per file.
/// Designed for AI agents that need to locate matches, not read every line.
fn print_compact(results: &[search::matcher::FileMatches]) {
    const MAX_FILES: usize = 25;
    const MAX_MATCHES_PER_FILE: usize = 5;
    const MAX_LINE_LEN: usize = 120;

    let total_matches: usize = results.iter().map(|f| f.match_count).sum();
    let total_files = results.len();

    println!("{} matches in {}F:", total_matches, total_files);
    println!();

    // Sort by match count descending (most relevant files first)
    let mut sorted: Vec<&search::matcher::FileMatches> = results.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.match_count));

    for (i, file_matches) in sorted.iter().enumerate() {
        if i >= MAX_FILES {
            let remaining_files = total_files - MAX_FILES;
            let remaining_matches: usize = sorted[MAX_FILES..].iter().map(|f| f.match_count).sum();
            println!(
                "... +{} files ({} matches)",
                remaining_files, remaining_matches
            );
            break;
        }

        // Shorten path if too long
        let path = &file_matches.path;
        let display_path = if path.len() > 50 {
            format!(
                ".../{}",
                path.rsplit('/')
                    .take(3)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("/")
            )
        } else {
            path.clone()
        };

        println!("[file] {} ({}):", display_path, file_matches.match_count);

        // Show first N matches (non-context only), truncated
        let mut shown = 0;
        for m in &file_matches.matches {
            if m.is_context {
                continue;
            }
            if shown >= MAX_MATCHES_PER_FILE {
                let remaining = file_matches
                    .match_count
                    .saturating_sub(MAX_MATCHES_PER_FILE);
                println!("  +{}", remaining);
                break;
            }
            let line_text = String::from_utf8_lossy(&m.line);
            let line_text = line_text.trim();
            let truncated = if line_text.len() > MAX_LINE_LEN {
                format!("{}...", &line_text[..MAX_LINE_LEN])
            } else {
                line_text.to_string()
            };
            println!("  {:>4}: {}", m.line_number, truncated);
            shown += 1;
        }
        println!();
    }
}

fn search_command_label(opts: &SearchOpts) -> String {
    let mut parts = vec![
        "ig".to_string(),
        "search".to_string(),
        opts.pattern.to_string(),
    ];
    parts.extend(opts.paths.iter().map(|p| (*p).to_string()));
    parts.join(" ")
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
            // Path equals root → no filter at all (don't push "" or "/",
            // both of which would either match nothing or be ambiguous).
            if rel_str.is_empty() {
                continue;
            }
            // For directories, ensure trailing slash so starts_with works as prefix
            if base.is_dir() && !rel_str.ends_with('/') {
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
