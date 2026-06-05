//! Rule content generators for AI tools (Windsurf, Cline).

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
