use clap::{Parser, Subcommand};

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

    /// Only print count of matching lines per file (rg -c parity)
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

    /// Show line numbers (always on, accepted for grep/rg compatibility)
    #[arg(short = 'n', long = "line-number", global = true)]
    pub line_number: bool,

    /// Match whole words only (wraps pattern with \b)
    #[arg(short = 'w', long, global = true)]
    pub word_regexp: bool,

    /// Treat pattern as fixed string (not regex)
    #[arg(short = 'F', long, global = true)]
    pub fixed_strings: bool,

    /// Output results as JSON lines (for AI agents)
    #[arg(long, global = true)]
    pub json: bool,

    /// Compact output: summary header + truncated matches (token-optimized for AI agents)
    #[arg(long, global = true)]
    pub compact: bool,

    /// Disable default directory exclusions
    #[arg(long, global = true)]
    pub no_default_excludes: bool,

    /// Max file size in bytes (default: 1MB, 0 = no limit)
    #[arg(long, global = true)]
    pub max_file_size: Option<u64>,

    /// Return only the top N files, ranked by BM25 relevance
    /// (tf × IDF × length normalisation). Keeps the most informative files
    /// when the raw match count would blow the context budget.
    #[arg(long, global = true, value_name = "N")]
    pub top: Option<usize>,

    /// Ultra-compact output mode — one-liner per item, no snippets,
    /// aggressive truncation. Token-optimised for AI agents that need
    /// the minimum signal possible.
    #[arg(short = 'u', long, global = true)]
    pub ultra_compact: bool,

    /// Verbosity (-v / -vv / -vvv). 0 = quiet (default), 3 = trace.
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Explain the action ig is about to take (rewrites, filter selection,
    /// permission verdict, …) without changing behaviour. Wired piecewise
    /// over subsequent PRs.
    #[arg(long, global = true)]
    pub explain: bool,

    /// Expand the query with learned co-occurring tokens (PMI).
    /// `ig --semantic error` also finds lines mentioning `catch`, `throw`,
    /// `Exception`, etc. — synonyms learned from THIS repo, no ML model.
    /// Requires a fresh index (`ig index`).
    #[arg(long, global = true)]
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

    /// List project files (respects .gitignore and excludes)
    Files {
        /// Directory to list (default: current dir)
        path: Option<String>,

        /// Tree-compressed output (group files by directory)
        #[arg(long)]
        compact: bool,
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

    /// Audit installed hooks and recent permission verdicts
    HookAudit {
        /// Number of past days to scan for the verdict histogram
        #[arg(long, default_value = "7")]
        since: u32,
        /// Emit machine-readable JSON instead of a human table
        #[arg(long)]
        json: bool,
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

        /// Output format: `text` (default) or `json`
        #[arg(long, default_value = "text")]
        format: String,

        /// Scan all available history, ignoring --since (no time cutoff)
        #[arg(long)]
        all: bool,
    },

    /// Generate shell completions (bash, zsh, fish, powershell)
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Configure AI CLI agents (Claude Code, Codex, Gemini...) to use ig.
    /// `init` is accepted as a synonym for RTK-muscle-memory compatibility.
    #[command(alias = "init")]
    Setup {
        /// Target a specific agent (all, claude, codex, cursor, copilot,
        /// gemini, opencode, windsurf, cline, hermes, kilocode, antigravity).
        #[arg(long, default_value = "all")]
        agent: String,

        /// Show what would be configured without writing any files
        #[arg(long)]
        dry_run: bool,

        /// Suppress the banner and skip already-up-to-date lines.
        /// Useful for post-`ig update` re-sync where most agents are
        /// already configured and only drifted entries should surface.
        #[arg(long, short)]
        quiet: bool,

        /// Install only the hook scripts / settings.json patches, skip
        /// the rules files (CLAUDE.md, AGENTS.md, …).
        #[arg(long)]
        hook_only: bool,

        /// Create missing agent config dirs (default: only patch existing ones).
        #[arg(long)]
        auto_patch: bool,

        /// Refuse to patch existing settings.json files even if they're stale.
        #[arg(long)]
        no_patch: bool,

        /// Show currently installed artifacts for the selected agent(s) and exit.
        #[arg(long)]
        show: bool,

        /// Uninstall ig artifacts for the selected agent(s).
        #[arg(long)]
        uninstall: bool,

        /// Also import RTK filter files into ig's filter format.
        #[arg(long)]
        import_rtk: bool,
    },

    /// Remove all ig artifacts (hooks, configs, binary, daemons, tracking data)
    Uninstall {
        /// Show what would be removed without actually removing anything
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Update ig itself and/or refresh project indexes
    Update {
        /// Root directory to update (default: current project)
        path: Option<String>,

        /// Refresh project indexes instead of updating ig + agent config
        #[arg(long)]
        indexes: bool,

        /// With --indexes, discover and refresh every known indexed project under path
        #[arg(long)]
        all: bool,

        /// Only update ig + agent config, even if a path is provided
        #[arg(long)]
        self_only: bool,
    },

    /// Run a command with token-optimized output filtering
    #[command(alias = "proxy")]
    Run {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Manage the tee store (raw output of truncated / failed commands)
    Tee {
        #[command(subcommand)]
        op: TeeOp,
    },

    /// Run a command and show only errors/warnings
    Err {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Auto-detect test framework and run tests with compact output
    Test {
        /// Extra arguments passed to the test runner
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show JSON file (compact or schema-only)
    Json {
        /// JSON file to read
        file: String,

        /// Show schema instead of values (types + array counts)
        #[arg(long)]
        schema: bool,
    },

    /// Summarize project dependencies (Cargo.toml, package.json, go.mod...)
    Deps,

    /// Docker commands with compact output
    Docker {
        /// Docker subcommand and arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show environment variables (sensitive values masked)
    Env {
        /// Filter by variable name pattern
        pattern: Option<String>,
    },

    /// Ultra-condensed diff between two files
    Diff {
        /// First file
        file1: String,
        /// Second file
        file2: String,
    },

    /// Trust a project-local filter file
    Trust {
        /// Path to .ig/filters/*.toml file to trust
        file: Option<String>,

        /// List all trusted filter files
        #[arg(long)]
        list: bool,
    },

    /// Revoke trust for a project-local filter file
    Untrust {
        /// Path to filter file to untrust
        file: String,
    },

    /// Verify TOML filter inline tests
    Verify,

    /// Detect CLI correction patterns from Claude Code sessions
    Learn {
        /// Only scan sessions from the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,

        /// Maximum entries to show (default: 15)
        #[arg(long, default_value = "15")]
        limit: usize,

        /// Output format: `text` (default) or `json`
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Opt-in anonymous telemetry (off by default; no endpoint compiled into
    /// the public build, so `ping` is a hard no-op there).
    Telemetry {
        #[command(subcommand)]
        op: TelemetryOp,
    },

    /// Import RTK filter files into ig's filter format (one-shot translation)
    ImportRtk {
        /// Preview the translation without writing any files
        #[arg(long)]
        dry_run: bool,

        /// Accept prompts non-interactively (reserved; import is non-destructive)
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Show ig adoption across Claude Code sessions
    Session {
        /// Only scan sessions from the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,
    },

    /// Show token savings translated to API cost savings
    Economics {
        /// Only analyze the last N days (default: 30)
        #[arg(long, default_value = "30")]
        since: u32,
    },

    /// POC — OpenAI embeddings playground (opt-in, build with `--features embed-poc`).
    /// Without the feature, falls back to `ig search` (trigram, no API key).
    #[cfg(feature = "embed-poc")]
    #[command(hide = true)]
    EmbedPoc {
        #[command(subcommand)]
        op: EmbedPocOp,
    },

    /// Toggle the runtime embedding switch: `ig emb on|off|status`.
    /// Default: disabled. Independent of the `embed-poc` cargo feature.
    Emb {
        /// `on` / `off` / `status` (default: status)
        state: Option<String>,
    },

    /// Garbage-collect the XDG cache (~/.cache/ig/) — drop orphans and stale entries
    Gc {
        /// Also remove entries unused for more than N days
        #[arg(long, value_name = "N")]
        days: Option<u64>,

        /// Also prune least-recently-used entries until cache is under SIZE (e.g. 5GB)
        #[arg(long, value_name = "SIZE")]
        max_size: Option<String>,

        /// Show what would be removed without deleting anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Migrate the current project's `.ig/` to the XDG cache
    Migrate {
        /// Project root to migrate (default: current dir)
        path: Option<String>,

        /// Show what would be migrated without moving anything
        #[arg(long)]
        dry_run: bool,
    },

    /// List XDG cache entries (~/.cache/ig/) with size + last-used info
    CacheLs,

    /// Generate a .ignore file tailored to the detected project stack
    Autoignore {
        /// Directory to generate .ignore for (default: current dir)
        path: Option<std::path::PathBuf>,

        /// Overwrite an existing .ignore file
        #[arg(long)]
        force: bool,
    },

    /// Print the ig version (alias for `--version`)
    ///
    /// `ig <word>` is otherwise treated as a search shortcut — without this
    /// subcommand, `ig version` would silently search for the word "version".
    Version,

    // ---- PR #4 — per-tool Rust parsers (test runners) ----
    /// Run vitest with a compact JSON-parsed summary
    Vitest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run jest with a compact JSON-parsed summary
    Jest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run playwright tests with a compact JSON-parsed summary
    Playwright {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run pytest with a compact summary (XFAIL/XPASS surfaced)
    Pytest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run `cargo test --no-fail-fast` with a compact summary
    CargoTest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run `go test -json ./...` with NDJSON-aggregated summary
    GoTest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run rspec with JSON-parsed summary
    Rspec {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run rake (routes to rspec when project uses it)
    Rake {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    // ---- PR #4 — linters / formatters ----
    /// Run eslint with JSON-parsed compact output
    Eslint {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run biome check with JSON-parsed compact output
    Biome {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run tsc --noEmit with parsed diagnostic compaction
    Tsc {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run prettier --check and surface only files needing format
    Prettier {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run ruff check with JSON-parsed compact output
    Ruff {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run mypy and parse its text diagnostics
    Mypy {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run rubocop with JSON-parsed compact output
    Rubocop {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run golangci-lint with JSON-parsed compact output
    GolangciLint {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    // ---- PR #4 — build / package ----
    /// Run next (build/dev) and surface route summary or errors
    Next {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run prisma generate/migrate and surface success or errors
    Prisma {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run pnpm install/add/remove with progress suppressed
    Pnpm {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run npm install/run with progress suppressed
    Npm {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run pip install and surface only success or error blocks
    Pip {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    // ---- PR #5 — git platforms / cloud / system wrappers ----
    /// GitHub CLI wrapper with JSON-driven compaction
    Gh {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// GitLab CLI wrapper with JSON-driven compaction
    Glab {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Graphite CLI wrapper (stack summary)
    Gt {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// AWS CLI wrapper (per-service compact rendering)
    Aws {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// kubectl wrapper for get/logs/describe/apply
    Kubectl {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// psql wrapper (compact SELECT / DML summary)
    Psql {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// curl wrapper with body truncation + tee fallback
    Curl {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// wget wrapper (surfaces summary line only)
    Wget {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// System log wrapper (log show on macOS, journalctl on Linux)
    Log {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Summarize a file (word count + first/last lines + markdown outline)
    Summary {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// tree wrapper (capped depth, gitignore-aware)
    Tree {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// wc wrapper (tracked passthrough)
    Wc {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden — feed stdin through one of the PR #4 parsers (for testing).
    ///
    /// Usage: `cat raw.txt | ig __parse vitest --ultra-compact`.
    #[command(name = "__parse", hide = true)]
    InternalParse {
        /// Tool name (vitest, jest, pytest, cargo_test, go_test, rspec,
        /// eslint, biome, tsc, prettier, ruff, mypy, rubocop, golangci,
        /// gh-pr-list, gh-pr-view, gh-issue-list, gh-run-list,
        /// aws-sts, aws-ec2, aws-lambda, aws-ddb, aws-iam-roles,
        /// kubectl-get, kubectl-describe, kubectl-apply, kubectl-tail,
        /// glab-mr-list, glab-issue-list,
        /// psql, wget, log-condense, summary-md, gt-condense).
        tool: String,
    },
}

#[cfg(feature = "embed-poc")]
#[derive(Subcommand)]
pub enum EmbedPocOp {
    /// Embed a single text input and print the vector summary (Phase 1)
    Hello {
        /// Text to embed
        text: String,
    },
    /// Chunk + embed a directory, store JSON at .ig/poc-embeddings.json (Phase 2)
    Index {
        /// Directory to index (default: current dir)
        dir: Option<String>,
        /// Skip the y/N cost confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Inspect the local store (human-readable)
    Inspect {
        /// How many chunks to preview (default: 10)
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Cosine top-N search over the local store
    Search {
        /// Query in natural language
        query: String,
        /// Top-N results (default: 5)
        #[arg(long, default_value = "5")]
        top: usize,
    },
    /// Phase 3 — start a tiny_http JSON server (+ optional static SPA)
    Serve {
        /// Bind port (127.0.0.1 only — POC)
        #[arg(long, default_value = "7877")]
        port: u16,
        /// Path to a built SPA directory (e.g. `ui/dist`). If absent, a landing page is served.
        #[arg(long)]
        ui: Option<String>,
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

#[derive(Subcommand)]
pub enum TelemetryOp {
    /// Show consent status, last ping, and whether an endpoint is compiled in
    Status,
    /// Grant or deny telemetry consent
    Consent {
        /// Grant consent
        #[arg(long)]
        yes: bool,
        /// Deny consent
        #[arg(long)]
        no: bool,
    },
    /// Assemble and print the telemetry payload without sending it
    Test,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn gain_full_flag_parses() {
        let cli = Cli::try_parse_from(["ig", "gain", "--full"]).unwrap();
        match cli.command {
            Some(Commands::Gain { full, .. }) => assert!(full),
            _ => panic!("expected gain command"),
        }
    }
}
