//! Hook content generators for additional AI tools (Copilot, Windsurf, Cline).

/// Generate Copilot instructions content for .github/copilot-instructions.md
pub fn copilot_instructions() -> String {
    "# ig — Token-optimized CLI\n\n\
     Use `ig` for all code search and file operations:\n\
     - `ig \"pattern\" [path]` — search (trigram-indexed, sub-ms)\n\
     - `ig read <file>` — read file with compression\n\
     - `ig read -s <file>` — signatures only\n\
     - `ig read -a <file>` — aggressive compression\n\
     - `ig ls [path]` — compact directory listing\n\
     - `ig git status/log/diff` — compressed git output\n\
     - `ig run <command>` — run any command with output compression\n\
     - `ig test` — auto-detect and run tests with compact output\n\
     - `ig deps` — show project dependencies\n\
     - `ig err <command>` — show only errors/warnings\n\n\
     Never use raw `cat`, `grep`, `find`, `ls -la`, or `tree` — use ig equivalents.\n"
        .to_string()
}

/// Generate Windsurf rules content for .windsurfrules
pub fn windsurf_rules() -> String {
    "# ig — Token-optimized CLI for AI agents\n\n\
     ## Rules\n\
     - Use `ig \"pattern\"` instead of `grep`, `rg`, or `find` for code search\n\
     - Use `ig read <file>` instead of `cat` for file reading\n\
     - Use `ig ls` instead of `ls -la` for directory listing\n\
     - Use `ig git status/log/diff` instead of raw git commands\n\
     - Use `ig run <command>` to run any command with output compression\n\
     - Use `ig test` to run tests with compact output\n\
     - Use `ig err <command>` to see only errors/warnings\n"
        .to_string()
}

/// Generate Cline/Roo rules content for .clinerules
pub fn cline_rules() -> String {
    "# ig — Token-optimized CLI for AI agents\n\n\
     Use ig for all code search and file operations:\n\
     - Search: `ig \"pattern\" [path]` (trigram-indexed, sub-ms)\n\
     - Read: `ig read <file>` / `ig read -s <file>` (signatures) / `ig read -a <file>` (aggressive)\n\
     - List: `ig ls [path]`\n\
     - Git: `ig git status/log/diff`\n\
     - Run: `ig run <command>` (filtered output)\n\
     - Test: `ig test` (auto-detect framework)\n\
     - Errors: `ig err <command>` (errors only)\n\n\
     Never use raw cat, grep, find, ls, or tree.\n"
        .to_string()
}
