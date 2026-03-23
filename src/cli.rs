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

    /// Start a daemon that serves search queries via Unix socket
    Daemon {
        /// Directory to serve (default: current dir)
        path: Option<String>,
    },

    /// Send a search query to a running daemon
    Query {
        /// Regex pattern to search for
        pattern: String,

        /// Directory the daemon is serving (default: current dir)
        path: Option<String>,
    },
}
