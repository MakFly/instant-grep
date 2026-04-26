use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SearchMode {
    Normal,
    Compact,
    Top,
    Semantic,
}

#[derive(Args, Clone, Debug, Default)]
pub struct SearchFlags {
    /// Search preset for agent-oriented output
    #[arg(long, value_enum)]
    pub mode: Option<SearchMode>,

    /// Case-insensitive search
    #[arg(short = 'i', long)]
    pub ignore_case: bool,

    /// Lines of context after each match
    #[arg(short = 'A', long, default_value = "0")]
    pub after_context: usize,

    /// Lines of context before each match
    #[arg(short = 'B', long, default_value = "0")]
    pub before_context: usize,

    /// Lines of context before and after each match
    #[arg(short = 'C', long)]
    pub context: Option<usize>,

    /// Only print count of matches per file
    #[arg(short = 'c', long)]
    pub count: bool,

    /// Only print file paths with matches
    #[arg(short = 'l', long)]
    pub files_with_matches: bool,

    /// Skip index, force brute-force scan
    #[arg(long)]
    pub no_index: bool,

    /// Show search statistics
    #[arg(long)]
    pub stats: bool,

    /// Filter by file type (e.g., rs, ts, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Filter by glob pattern (e.g., "*.php")
    #[arg(short = 'g', long)]
    pub glob: Option<String>,

    /// Show line numbers (always on, accepted for grep/rg compatibility)
    #[arg(short = 'n', long = "line-number")]
    pub line_number: bool,

    /// Match whole words only (wraps pattern with \b)
    #[arg(short = 'w', long)]
    pub word_regexp: bool,

    /// Treat pattern as fixed string (not regex)
    #[arg(short = 'F', long)]
    pub fixed_strings: bool,

    /// Output results as JSON lines (for AI agents)
    #[arg(long)]
    pub json: bool,

    /// Compact output: summary header + truncated matches (token-optimized for AI agents)
    #[arg(long)]
    pub compact: bool,

    /// Disable default directory exclusions
    #[arg(long)]
    pub no_default_excludes: bool,

    /// Max file size in bytes (default: 1MB, 0 = no limit)
    #[arg(long)]
    pub max_file_size: Option<u64>,

    /// Return only the top N files, ranked by BM25 relevance
    #[arg(long, value_name = "N")]
    pub top: Option<usize>,

    /// Expand the query with learned co-occurring tokens (PMI)
    #[arg(long)]
    pub semantic: bool,
}

#[derive(Parser)]
#[command(name = "ig", version, about = "Trigram-indexed regex search")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Regex pattern to search for (shortcut: `ig "pattern"` = `ig search "pattern"`)
    #[arg(global = false)]
    pub pattern: Option<String>,

    /// Directories or files to search (default: current dir)
    #[arg(global = false, num_args = 0..)]
    pub paths: Vec<String>,

    #[command(flatten)]
    pub search: LegacySearchFlags,
}

#[derive(Args, Clone, Debug, Default)]
pub struct LegacySearchFlags {
    /// Search preset for agent-oriented output
    #[arg(long, value_enum, global = true, hide = true)]
    pub mode: Option<SearchMode>,

    /// Case-insensitive search
    #[arg(short = 'i', long, global = true, hide = true)]
    pub ignore_case: bool,

    /// Lines of context after each match
    #[arg(short = 'A', long, default_value = "0", global = true, hide = true)]
    pub after_context: usize,

    /// Lines of context before each match
    #[arg(short = 'B', long, default_value = "0", global = true, hide = true)]
    pub before_context: usize,

    /// Lines of context before and after each match
    #[arg(short = 'C', long, global = true, hide = true)]
    pub context: Option<usize>,

    /// Only print count of matches per file
    #[arg(short = 'c', long, global = true, hide = true)]
    pub count: bool,

    /// Only print file paths with matches
    #[arg(short = 'l', long, global = true, hide = true)]
    pub files_with_matches: bool,

    /// Skip index, force brute-force scan
    #[arg(long, global = true, hide = true)]
    pub no_index: bool,

    /// Show search statistics
    #[arg(long, global = true, hide = true)]
    pub stats: bool,

    /// Filter by file type (e.g., rs, ts, py)
    #[arg(short = 't', long = "type", global = true, hide = true)]
    pub file_type: Option<String>,

    /// Filter by glob pattern (e.g., "*.php")
    #[arg(short = 'g', long, global = true, hide = true)]
    pub glob: Option<String>,

    /// Show line numbers (always on, accepted for grep/rg compatibility)
    #[arg(short = 'n', long = "line-number", global = true, hide = true)]
    pub line_number: bool,

    /// Match whole words only (wraps pattern with \b)
    #[arg(short = 'w', long, global = true, hide = true)]
    pub word_regexp: bool,

    /// Treat pattern as fixed string (not regex)
    #[arg(short = 'F', long, global = true, hide = true)]
    pub fixed_strings: bool,

    /// Output results as JSON lines (for AI agents)
    #[arg(long, global = true, hide = true)]
    pub json: bool,

    /// Compact output: summary header + truncated matches (token-optimized for AI agents)
    #[arg(long, global = true, hide = true)]
    pub compact: bool,

    /// Disable default directory exclusions
    #[arg(long, global = true, hide = true)]
    pub no_default_excludes: bool,

    /// Max file size in bytes (default: 1MB, 0 = no limit)
    #[arg(long, global = true, hide = true)]
    pub max_file_size: Option<u64>,

    /// Return only the top N files, ranked by BM25 relevance
    #[arg(long, global = true, value_name = "N", hide = true)]
    pub top: Option<usize>,

    /// Expand the query with learned co-occurring tokens (PMI)
    #[arg(long, global = true, hide = true)]
    pub semantic: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search for a regex pattern (builds index if needed)
    Search {
        /// Regex pattern to search for
        pattern: String,

        /// Directories or files to search (default: current dir)
        #[arg(num_args = 0..)]
        paths: Vec<String>,

        #[command(flatten)]
        flags: SearchFlags,
    },

    /// Build or rebuild the trigram index
    Index {
        /// Directory to index (default: current dir)
        path: Option<String>,
    },

    /// Show index statistics
    Status {
        /// Directory to check (default: current dir)
        path: Option<String>,
    },

    /// Watch for file changes and rebuild index automatically
    #[command(hide = true)]
    Watch {
        /// Directory to watch (default: current dir)
        path: Option<String>,
    },

    /// Manage the search daemon (start/stop/status/install/uninstall)
    #[command(hide = true)]
    Daemon {
        /// Action: start, stop, status, install, uninstall (default: start in foreground)
        action: Option<String>,

        /// Directory to serve (default: current dir)
        path: Option<String>,
    },

    /// List project files (respects .gitignore and excludes)
    Files {
        /// Directory to list (default: current dir)
        path: Option<String>,

        /// Tree-compressed output (group files by directory)
        #[arg(long)]
        compact: bool,
    },

    /// Extract symbol definitions (functions, classes, structs...)
    #[command(hide = true)]
    Symbols {
        /// Directory to scan (default: current dir)
        path: Option<String>,
    },

    /// Show the full code block containing a specific line
    #[command(hide = true)]
    Context {
        /// File path
        file: String,
        /// Line number to show context for
        line: usize,
    },

    /// Read a file with optional signatures-only mode
    Read {
        /// File path to read
        file: String,

        /// Show only imports and symbol signatures
        #[arg(short = 's', long)]
        signatures: bool,

        /// Aggressive compression (strip comments, function bodies, string literals)
        #[arg(short = 'a', long)]
        aggressive: bool,

        /// Max output tokens (1 token ≈ 4 chars). Implies -a. Uses entropy scoring to keep the most informative lines.
        #[arg(short = 'b', long)]
        budget: Option<usize>,

        /// Boost relevance of lines matching this pattern (use with -b for best results)
        #[arg(short = 'r', long)]
        relevant: Option<String>,

        /// Show only git-changed lines with enclosing context
        #[arg(short = 'd', long)]
        delta: bool,

        /// Raw output — no line numbers, no colors (byte-for-byte identical to cat)
        #[arg(short = 'p', long)]
        plain: bool,
    },

    /// Show 2-line smart summary for each file
    #[command(hide = true)]
    Smart {
        /// File or directory to summarize (default: current dir)
        path: Option<String>,
    },

    /// Generate .ig/context.md (tree + smart summaries + symbols)
    #[command(hide = true)]
    Pack {
        /// Directory to pack (default: current dir)
        path: Option<String>,
    },

    /// Compact directory listing (token-optimized for AI agents)
    #[command(hide = true)]
    Ls {
        /// Directory to list (default: current dir)
        path: Option<String>,
    },

    /// Token-compressed git output (status, log, diff, branch, show)
    Git {
        /// Git subcommand and arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Rewrite a shell command to its ig equivalent (used by hooks)
    #[command(hide = true)]
    Rewrite {
        /// The command to rewrite
        command: String,
    },

    /// Show token savings dashboard
    Gain {
        /// Clear tracking history
        #[arg(long)]
        clear: bool,

        /// Show the full "By Command" table instead of the top 15
        #[arg(long)]
        full: bool,

        /// Show individual command history
        #[arg(short = 'H', long)]
        history: bool,

        /// Output as JSON (for scripting)
        #[arg(long)]
        json: bool,

        /// Filter to current project only
        #[arg(short = 'p', long)]
        project: bool,

        /// Show ASCII graph of daily savings (last 14 days)
        #[arg(long)]
        graph: bool,

        /// Show monthly quota savings estimate
        #[arg(short = 'q', long)]
        quota: bool,

        /// Subscription tier for quota calc: pro, 5x, 20x
        #[arg(long, default_value = "20x")]
        tier: String,

        /// Show daily breakdown
        #[arg(short = 'd', long)]
        daily: bool,

        /// Show weekly breakdown
        #[arg(long)]
        weekly: bool,

        /// Show monthly breakdown
        #[arg(short = 'm', long)]
        monthly: bool,

        /// Discover missed savings from Claude Code sessions
        #[arg(long)]
        discover: bool,

        /// Days to scan for --discover (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,

        /// Show commands from shell history that should have gone through ig
        #[arg(long)]
        missed: bool,

        /// Compare two periods (this-week, last-week, this-month, last-month,
        /// this-day, last-day). Format: "this-week:last-week".
        #[arg(long, value_name = "PERIODS")]
        compare: Option<String>,
    },

    /// Execute a command without ig filtering (debug/passthrough mode)
    #[command(hide = true)]
    Raw {
        /// The command to execute raw
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Discover missed token-saving opportunities across agent sessions and
    /// shell history. Reports commands that could have gone through `ig run`
    /// but didn't.
    #[command(hide = true)]
    Discover {
        /// Only scan sessions/history from the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,

        /// Maximum entries to show per section (default: 15)
        #[arg(long, default_value = "15")]
        limit: usize,

        /// Also scan ~/.zsh_history and ~/.bash_history for missed cmds
        #[arg(long)]
        shell: bool,
    },

    /// Generate shell completions (bash, zsh, fish, powershell)
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Configure AI CLI agents (Claude Code, Codex, Gemini...) to use ig
    Setup {
        /// Show what would be configured without writing any files
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove all ig artifacts (hooks, configs, binary, daemons, tracking data)
    #[command(hide = true)]
    Uninstall {
        /// Show what would be removed without actually removing anything
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Update ig to the latest version
    #[command(hide = true)]
    Update,

    /// Send a search query to a running daemon
    #[command(hide = true)]
    Query {
        /// Regex pattern to search for
        pattern: String,

        /// Directory the daemon is serving (default: current dir)
        path: Option<String>,
    },

    /// Run a command with token-optimized output filtering
    #[command(alias = "proxy")]
    Run {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Manage the tee store (raw output of truncated / failed commands)
    #[command(hide = true)]
    Tee {
        #[command(subcommand)]
        op: TeeOp,
    },

    /// Run a command and show only errors/warnings
    #[command(hide = true)]
    Err {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Auto-detect test framework and run tests with compact output
    #[command(hide = true)]
    Test {
        /// Extra arguments passed to the test runner
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show JSON file (compact or schema-only)
    #[command(hide = true)]
    Json {
        /// JSON file to read
        file: String,

        /// Show schema instead of values (types + array counts)
        #[arg(long)]
        schema: bool,
    },

    /// Summarize project dependencies (Cargo.toml, package.json, go.mod...)
    #[command(hide = true)]
    Deps,

    /// Docker commands with compact output
    #[command(hide = true)]
    Docker {
        /// Docker subcommand and arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show environment variables (sensitive values masked)
    #[command(hide = true)]
    Env {
        /// Filter by variable name pattern
        pattern: Option<String>,
    },

    /// Ultra-condensed diff between two files
    #[command(hide = true)]
    Diff {
        /// First file
        file1: String,
        /// Second file
        file2: String,
    },

    /// Trust a project-local filter file
    #[command(hide = true)]
    Trust {
        /// Path to .ig/filters/*.toml file to trust
        file: Option<String>,

        /// List all trusted filter files
        #[arg(long)]
        list: bool,
    },

    /// Revoke trust for a project-local filter file
    #[command(hide = true)]
    Untrust {
        /// Path to filter file to untrust
        file: String,
    },

    /// Verify TOML filter inline tests
    #[command(hide = true)]
    Verify,

    /// Detect CLI correction patterns from Claude Code sessions
    #[command(hide = true)]
    Learn {
        /// Only scan sessions from the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,

        /// Maximum entries to show (default: 15)
        #[arg(long, default_value = "15")]
        limit: usize,
    },

    /// Show ig adoption across Claude Code sessions
    #[command(hide = true)]
    Session {
        /// Only scan sessions from the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,
    },

    /// Show token savings translated to API cost savings
    #[command(hide = true)]
    Economics {
        /// Only analyze the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,
    },
}

#[derive(Subcommand)]
pub enum TeeOp {
    /// Print the raw content of a tee entry
    Show {
        /// Tee id (from `ig tee list`)
        id: String,
    },
    /// List tee entries, newest first
    List,
    /// Delete every tee entry
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    fn help_for(args: &[&str]) -> String {
        let mut cmd = Cli::command();
        for arg in args {
            cmd = cmd.find_subcommand_mut(arg).unwrap().clone();
        }
        let mut buf = Vec::new();
        cmd.write_long_help(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn gain_full_flag_parses() {
        let cli = Cli::try_parse_from(["ig", "gain", "--full"]).unwrap();
        match cli.command {
            Some(Commands::Gain { full, .. }) => assert!(full),
            _ => panic!("expected gain command"),
        }
    }

    #[test]
    fn root_help_shows_core_commands_only() {
        let help = help_for(&[]);
        for cmd in [
            "search", "index", "status", "read", "files", "git", "run", "setup", "gain",
        ] {
            assert!(help.contains(cmd), "missing {cmd} in root help:\n{help}");
        }
        for hidden in [
            "daemon",
            "watch",
            "symbols",
            "context",
            "discover",
            "economics",
            "verify",
        ] {
            assert!(
                !help.contains(hidden),
                "hidden command {hidden} leaked into root help:\n{help}"
            );
        }
    }

    #[test]
    fn non_search_help_hides_search_options() {
        for cmd in ["read", "gain", "setup"] {
            let help = help_for(&[cmd]);
            for flag in ["--semantic", "--top", "--no-index", "--glob"] {
                assert!(
                    !help.contains(flag),
                    "{cmd} help should not show {flag}:\n{help}"
                );
            }
        }
    }

    #[test]
    fn search_help_shows_search_options_and_mode() {
        let help = help_for(&["search"]);
        for flag in ["--mode", "--semantic", "--top", "--no-index", "--glob"] {
            assert!(help.contains(flag), "search help missing {flag}:\n{help}");
        }
    }

    #[test]
    fn hidden_commands_remain_parseable() {
        let cli = Cli::try_parse_from(["ig", "discover", "--since", "7"]).unwrap();
        match cli.command {
            Some(Commands::Discover { since, .. }) => assert_eq!(since, 7),
            _ => panic!("expected discover command"),
        }
    }

    #[test]
    fn legacy_global_search_flags_remain_parseable() {
        let cli = Cli::try_parse_from(["ig", "read", "--semantic", "src/main.rs"]).unwrap();
        assert!(cli.search.semantic);
        match cli.command {
            Some(Commands::Read { file, .. }) => assert_eq!(file, "src/main.rs"),
            _ => panic!("expected read command"),
        }

        let cli = Cli::try_parse_from(["ig", "setup", "-C", "3", "--dry-run"]).unwrap();
        assert_eq!(cli.search.context, Some(3));
        match cli.command {
            Some(Commands::Setup { dry_run }) => assert!(dry_run),
            _ => panic!("expected setup command"),
        }
    }
}
