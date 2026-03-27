use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ig", version, about = "Trigram-indexed regex search")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Regex pattern to search for (shortcut: `ig "pattern"` = `ig search "pattern"`)
    #[arg(global = false)]
    pub pattern: Option<String>,

    /// Directory or file to search (default: current dir)
    #[arg(global = false)]
    pub path: Option<String>,

    /// Case-insensitive search
    #[arg(short = 'i', long, global = true)]
    pub ignore_case: bool,

    /// Lines of context after each match
    #[arg(short = 'A', long, default_value = "0", global = true)]
    pub after_context: usize,

    /// Lines of context before each match
    #[arg(short = 'B', long, default_value = "0", global = true)]
    pub before_context: usize,

    /// Lines of context before and after each match
    #[arg(short = 'C', long, global = true)]
    pub context: Option<usize>,

    /// Only print count of matches per file
    #[arg(short = 'c', long, global = true)]
    pub count: bool,

    /// Only print file paths with matches
    #[arg(short = 'l', long, global = true)]
    pub files_with_matches: bool,

    /// Skip index, force brute-force scan
    #[arg(long, global = true)]
    pub no_index: bool,

    /// Show search statistics
    #[arg(long, global = true)]
    pub stats: bool,

    /// Filter by file type (e.g., rs, ts, py)
    #[arg(short = 't', long = "type", global = true)]
    pub file_type: Option<String>,

    /// Filter by glob pattern (e.g., "*.php")
    #[arg(short = 'g', long, global = true)]
    pub glob: Option<String>,

    /// Match whole words only (wraps pattern with \b)
    #[arg(short = 'w', long, global = true)]
    pub word_regexp: bool,

    /// Treat pattern as fixed string (not regex)
    #[arg(short = 'F', long, global = true)]
    pub fixed_strings: bool,

    /// Output results as JSON lines (for AI agents)
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable default directory exclusions
    #[arg(long, global = true)]
    pub no_default_excludes: bool,

    /// Max file size in bytes (default: 1MB, 0 = no limit)
    #[arg(long, global = true)]
    pub max_file_size: Option<u64>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search for a regex pattern (builds index if needed)
    Search {
        /// Regex pattern to search for
        pattern: String,

        /// Directory or file to search (default: current dir)
        path: Option<String>,
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
    Watch {
        /// Directory to watch (default: current dir)
        path: Option<String>,
    },

    /// Manage the search daemon (start/stop/status/install/uninstall)
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
    },

    /// Extract symbol definitions (functions, classes, structs...)
    Symbols {
        /// Directory to scan (default: current dir)
        path: Option<String>,
    },

    /// Show the full code block containing a specific line
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
    },

    /// Show 2-line smart summary for each file
    Smart {
        /// File or directory to summarize (default: current dir)
        path: Option<String>,
    },

    /// Generate .ig/context.md (tree + smart summaries + symbols)
    Pack {
        /// Directory to pack (default: current dir)
        path: Option<String>,
    },

    /// Compact directory listing (token-optimized for AI agents)
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

        /// Show individual command history
        #[arg(short = 'H', long)]
        history: bool,

        /// Output as JSON (for scripting)
        #[arg(long)]
        json: bool,
    },

    /// Execute a command without ig filtering (debug/passthrough mode)
    #[command(hide = true)]
    Proxy {
        /// The command to execute raw
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Generate shell completions (bash, zsh, fish, powershell)
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Configure AI CLI agents (Claude Code, Codex, Gemini...) to use ig
    Setup,

    /// Send a search query to a running daemon
    Query {
        /// Regex pattern to search for
        pattern: String,

        /// Directory the daemon is serving (default: current dir)
        path: Option<String>,
    },
}
