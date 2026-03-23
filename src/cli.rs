use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ig", version, about = "Trigram-indexed regex search")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search for a regex pattern (builds index if needed)
    Search {
        /// Regex pattern to search for
        pattern: String,

        /// Directory or file to search (default: current dir)
        path: Option<String>,

        /// Lines of context after each match
        #[arg(short = 'A', long, default_value = "0")]
        after_context: usize,

        /// Lines of context before each match
        #[arg(short = 'B', long, default_value = "0")]
        before_context: usize,

        /// Lines of context before and after each match
        #[arg(short = 'C', long)]
        context: Option<usize>,

        /// Case-insensitive search
        #[arg(short = 'i', long)]
        ignore_case: bool,

        /// Only print count of matches per file
        #[arg(short = 'c', long)]
        count: bool,

        /// Only print file paths with matches
        #[arg(short = 'l', long)]
        files_with_matches: bool,

        /// Skip index, force brute-force scan
        #[arg(long)]
        no_index: bool,

        /// Show search statistics (candidates vs total, timing)
        #[arg(long)]
        stats: bool,

        /// Filter by file type (e.g., rs, ts, py)
        #[arg(short = 't', long = "type")]
        file_type: Option<String>,

        /// Filter by glob pattern (e.g., "*.php")
        #[arg(short = 'g', long)]
        glob: Option<String>,

        /// Output results as JSON lines (for AI agents)
        #[arg(long)]
        json: bool,

        /// Disable default directory exclusions
        #[arg(long)]
        no_default_excludes: bool,

        /// Max file size in bytes (default: 1MB, 0 = no limit)
        #[arg(long)]
        max_file_size: Option<u64>,
    },

    /// Build or rebuild the trigram index
    Index {
        /// Directory to index (default: current dir)
        path: Option<String>,

        /// Disable default directory exclusions
        #[arg(long)]
        no_default_excludes: bool,

        /// Max file size in bytes (default: 1MB, 0 = no limit)
        #[arg(long)]
        max_file_size: Option<u64>,
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

        /// Disable default directory exclusions
        #[arg(long)]
        no_default_excludes: bool,
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

        /// Case-insensitive search
        #[arg(short = 'i', long)]
        ignore_case: bool,
    },
}
