//! Shared parser output types.

/// Rendering mode for `TokenFormatter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FormatMode {
    /// Default compact format — readable, includes basic context.
    #[default]
    Compact,
    /// Ultra-compact — one line per failure, no snippets, truncated.
    /// Used when `--ultra-compact / -u` is passed.
    Ultra,
}

/// A typed test-run summary, produced by any test-runner parser
/// (vitest, jest, pytest, cargo test, …).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    pub failures: Vec<TestFailure>,
}

/// A single failing test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestFailure {
    pub name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    /// Optional source/diff snippet. Always dropped in `FormatMode::Ultra`.
    pub snippet: Option<String>,
}

/// A typed lint/static-analysis summary produced by any linter parser
/// (eslint, biome, tsc, ruff, mypy, rubocop, golangci-lint, …).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintResult {
    pub errors: u32,
    pub warnings: u32,
    pub files: Vec<LintMessage>,
}

/// One lint diagnostic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintMessage {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub rule: String,
    pub message: String,
    /// `"error"` / `"warning"` / `"info"`. Linters normalise their levels to
    /// one of these three strings.
    pub severity: String,
}
