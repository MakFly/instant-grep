//! Command rewriting engine — intercepts shell commands and maps them to ig equivalents.
//! Used by the PreToolUse hook to transparently redirect cat/grep/ls/tree/find to ig.
//!
//! Exit codes (same protocol as RTK):
//!   0 + stdout  → rewrite found, auto-allow
//!   1           → no rewrite, passthrough
//!   2           → deny, reason on stderr
//!   3 + stdout  → rewrite found, require user confirmation

use std::process;

pub enum RewriteResult {
    Rewrite(String), // exit 0 — rewrite + auto-allow
    Passthrough,     // exit 1 — no rewrite
    Deny(String),    // exit 2 — blocked command, reason on stderr
    Ask(String),     // exit 3 — rewrite but require user confirmation
}

pub fn run_rewrite(command: &str) {
    match classify_command(command) {
        RewriteResult::Rewrite(cmd) => {
            print!("{}", cmd);
            process::exit(0);
        }
        RewriteResult::Passthrough => {
            process::exit(1);
        }
        RewriteResult::Deny(reason) => {
            eprintln!("DENY: {}", reason);
            process::exit(2);
        }
        RewriteResult::Ask(cmd) => {
            print!("{}", cmd);
            process::exit(3);
        }
    }
}

pub fn classify_command(cmd: &str) -> RewriteResult {
    let trimmed = cmd.trim();

    // --- Deny rules (checked before any rewrite) ---
    if is_deny_git_reset_hard(trimmed) {
        return RewriteResult::Deny("Destructive: resets all changes".to_string());
    }
    if is_deny_git_clean(trimmed) {
        return RewriteResult::Deny("Destructive: deletes untracked files".to_string());
    }
    if is_deny_rm_rf(trimmed) {
        return RewriteResult::Deny("Destructive: recursive force delete".to_string());
    }

    // --- Ask rules ---
    if is_ask_git_push_force(trimmed) {
        return RewriteResult::Ask(trimmed.to_string());
    }

    // --- Rewrite rules ---
    match try_rewrite(trimmed) {
        Some(rewritten) => RewriteResult::Rewrite(rewritten),
        None => RewriteResult::Passthrough,
    }
}

fn is_deny_git_reset_hard(cmd: &str) -> bool {
    let parts: Vec<&str> = shell_split(cmd);
    // git reset --hard [ref]
    parts.len() >= 3 && parts[0] == "git" && parts[1] == "reset" && parts.contains(&"--hard")
}

fn is_deny_git_clean(cmd: &str) -> bool {
    let parts: Vec<&str> = shell_split(cmd);
    if parts.len() < 2 || parts[0] != "git" || parts[1] != "clean" {
        return false;
    }
    // Match `git clean -f` and `git clean -fd` (and combined flags like -fdn etc.)
    parts
        .iter()
        .skip(2)
        .any(|p| p.starts_with('-') && !p.starts_with("--") && p.contains('f'))
}

fn is_deny_rm_rf(cmd: &str) -> bool {
    let parts: Vec<&str> = shell_split(cmd);
    if parts.is_empty() || parts[0] != "rm" {
        return false;
    }
    let has_recursive = parts.iter().any(|p| {
        *p == "-r"
            || *p == "-R"
            || *p == "--recursive"
            || (p.starts_with('-') && !p.starts_with("--") && (p.contains('r') || p.contains('R')))
    });
    let has_force = parts.iter().any(|p| {
        *p == "-f"
            || *p == "--force"
            || (p.starts_with('-') && !p.starts_with("--") && p.contains('f'))
    });
    if !has_recursive || !has_force {
        return false;
    }
    // Check for dangerous targets
    let dangerous_targets = ["/", ".", "~"];
    parts
        .iter()
        .filter(|p| !p.starts_with('-'))
        .skip(1) // skip "rm"
        .any(|p| dangerous_targets.contains(p))
}

fn is_ask_git_push_force(cmd: &str) -> bool {
    let parts: Vec<&str> = shell_split(cmd);
    if parts.len() < 2 || parts[0] != "git" || parts[1] != "push" {
        return false;
    }
    parts
        .iter()
        .skip(2)
        .any(|p| *p == "--force" || *p == "-f" || *p == "--force-with-lease")
}

fn try_rewrite(cmd: &str) -> Option<String> {
    // Skip empty or compound commands (pipes, &&, ||, ;)
    if cmd.is_empty()
        || cmd.contains('|')
        || cmd.contains("&&")
        || cmd.contains("||")
        || cmd.contains(';')
    {
        return None;
    }

    let parts: Vec<&str> = shell_split(cmd);
    if parts.is_empty() {
        return None;
    }

    let bin = parts[0];
    match bin {
        "cat" => rewrite_cat(&parts),
        "head" => rewrite_head(&parts),
        "tail" => rewrite_tail(&parts),
        "grep" | "egrep" | "fgrep" => rewrite_grep(&parts),
        "rg" => rewrite_rg(&parts),
        "tree" => rewrite_tree(&parts),
        "find" => rewrite_find(&parts),
        "ls" => rewrite_ls(&parts),
        "git" => rewrite_git(&parts),
        _ => None,
    }
}

/// cat file → ig read file
fn rewrite_cat(parts: &[&str]) -> Option<String> {
    // Only rewrite simple `cat file` (no flags like -n, -A, etc.)
    if parts.len() == 2 && !parts[1].starts_with('-') {
        Some(format!("ig read {}", parts[1]))
    } else {
        None
    }
}

/// head -N file → ig read file (first N lines shown by default)
fn rewrite_head(parts: &[&str]) -> Option<String> {
    match parts.len() {
        2 if !parts[1].starts_with('-') => {
            // head file → ig read file
            Some(format!("ig read {}", parts[1]))
        }
        3 => {
            // head -N file or head -n N file
            let file = parts.last()?;
            if file.starts_with('-') {
                return None;
            }
            Some(format!("ig read {}", file))
        }
        _ => None,
    }
}

/// tail -N file → ig read file
fn rewrite_tail(parts: &[&str]) -> Option<String> {
    match parts.len() {
        2 if !parts[1].starts_with('-') => Some(format!("ig read {}", parts[1])),
        3 => {
            let file = parts.last()?;
            if file.starts_with('-') {
                return None;
            }
            Some(format!("ig read {}", file))
        }
        _ => None,
    }
}

/// grep -r pattern dir → ig "pattern" dir
fn rewrite_grep(parts: &[&str]) -> Option<String> {
    // Only intercept recursive grep (code search)
    let has_recursive = parts.iter().any(|p| {
        *p == "-r"
            || *p == "-R"
            || *p == "--recursive"
            || (p.starts_with('-') && !p.starts_with("--") && (p.contains('r') || p.contains('R')))
    });

    if !has_recursive {
        return None;
    }

    // Extract pattern and path
    let mut pattern = None;
    let mut path = None;
    let mut skip_next = false;

    for part in parts.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if part.starts_with('-') {
            // Flags like -e, --include take a value
            if *part == "-e" || *part == "--include" || *part == "--exclude" {
                skip_next = true;
            }
            continue;
        }
        if pattern.is_none() {
            pattern = Some(*part);
        } else if path.is_none() {
            path = Some(*part);
        }
    }

    let pattern = pattern?;
    let case_flag = if parts
        .iter()
        .any(|p| *p == "-i" || (p.starts_with('-') && !p.starts_with("--") && p.contains('i')))
    {
        " -i"
    } else {
        ""
    };

    match path {
        Some(p) if p != "." => Some(format!("ig{} \"{}\" {}", case_flag, pattern, p)),
        _ => Some(format!("ig{} \"{}\"", case_flag, pattern)),
    }
}

/// rg pattern [path] → ig "pattern" [path]
fn rewrite_rg(parts: &[&str]) -> Option<String> {
    let mut pattern = None;
    let mut path = None;
    let mut case_flag = "";
    let mut type_filter: Option<&str> = None;
    let mut skip_next = false;
    let mut next_is_type = false;

    for part in parts.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if next_is_type {
            type_filter = Some(*part);
            next_is_type = false;
            continue;
        }
        if *part == "-i" || *part == "--ignore-case" {
            case_flag = " -i";
            continue;
        }
        if part.starts_with('-') {
            if *part == "-t" || *part == "--type" {
                next_is_type = true;
            } else if *part == "-g" || *part == "--glob" {
                skip_next = true;
            }
            continue;
        }
        if pattern.is_none() {
            pattern = Some(*part);
        } else if path.is_none() {
            path = Some(*part);
        }
    }

    let pattern = pattern?;
    let type_arg = match type_filter {
        Some(t) => format!(" --type {}", t),
        None => String::new(),
    };
    match path {
        Some(p) => Some(format!("ig{}{} \"{}\" {}", case_flag, type_arg, pattern, p)),
        None => Some(format!("ig{}{} \"{}\"", case_flag, type_arg, pattern)),
    }
}

/// tree → cat .ig/tree.txt (if exists) or ig ls
fn rewrite_tree(_parts: &[&str]) -> Option<String> {
    // Always rewrite tree (with or without flags like -L N -I pattern)
    Some("cat .ig/tree.txt 2>/dev/null || ig ls".to_string())
}

/// find . -name "*.ts" → ig files --glob "*.ts"
fn rewrite_find(parts: &[&str]) -> Option<String> {
    // Only rewrite find with -name pattern
    let name_idx = parts.iter().position(|p| *p == "-name" || *p == "-iname")?;
    let pattern = parts.get(name_idx + 1)?;

    // Skip if there are destructive or complex action flags
    if parts
        .iter()
        .any(|p| *p == "-exec" || *p == "-delete" || *p == "-print0")
    {
        return None;
    }

    // Allow -type f (file-only filter — always safe to ignore since ig only indexes files)
    // Reject other -type values (d, l, etc.)
    let mut i = 1;
    while i < parts.len() {
        if parts[i] == "-type"
            && let Some(val) = parts.get(i + 1)
        {
            if *val != "f" {
                return None;
            }
            i += 2;
            continue;
        }
        i += 1;
    }

    Some(format!("ig files --glob {}", pattern))
}

/// ls [dir] → ig ls [dir]
fn rewrite_ls(parts: &[&str]) -> Option<String> {
    // Collect non-flag args
    let args: Vec<&str> = parts
        .iter()
        .skip(1)
        .filter(|p| !p.starts_with('-'))
        .copied()
        .collect();

    match args.len() {
        0 => Some("ig ls".to_string()),
        1 => Some(format!("ig ls {}", args[0])),
        _ => None, // Multiple paths — don't rewrite
    }
}

/// git status/log/diff/branch/show → ig git <subcmd> [args]
/// Destructive commands (push, reset, checkout, clean, rebase, merge, commit) are NOT rewritten.
fn rewrite_git(parts: &[&str]) -> Option<String> {
    if parts.len() < 2 {
        return None;
    }
    let subcmd = parts[1];
    // Only rewrite read-only git subcommands
    match subcmd {
        "status" | "log" | "diff" | "branch" | "show" => {
            let args = parts[2..].join(" ");
            if args.is_empty() {
                Some(format!("ig git {}", subcmd))
            } else {
                Some(format!("ig git {} {}", subcmd, args))
            }
        }
        _ => None, // Don't rewrite destructive/write commands
    }
}

/// Simple shell-like splitting (handles quotes minimally)
fn shell_split(cmd: &str) -> Vec<&str> {
    cmd.split_whitespace().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Rewrite tests (via classify_command) ---

    #[test]
    fn test_rewrite_cat() {
        assert!(matches!(
            classify_command("cat src/main.rs"),
            RewriteResult::Rewrite(s) if s == "ig read src/main.rs"
        ));
        assert!(matches!(
            classify_command("cat -n src/main.rs"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_head() {
        assert!(matches!(
            classify_command("head src/main.rs"),
            RewriteResult::Rewrite(s) if s == "ig read src/main.rs"
        ));
        assert!(matches!(
            classify_command("head -50 src/main.rs"),
            RewriteResult::Rewrite(s) if s == "ig read src/main.rs"
        ));
    }

    #[test]
    fn test_rewrite_tail() {
        assert!(matches!(
            classify_command("tail src/main.rs"),
            RewriteResult::Rewrite(s) if s == "ig read src/main.rs"
        ));
        assert!(matches!(
            classify_command("tail -20 src/main.rs"),
            RewriteResult::Rewrite(s) if s == "ig read src/main.rs"
        ));
    }

    #[test]
    fn test_rewrite_grep_recursive() {
        assert!(matches!(
            classify_command("grep -rn useState src/"),
            RewriteResult::Rewrite(s) if s == "ig \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("grep -ri pattern ."),
            RewriteResult::Rewrite(s) if s == "ig -i \"pattern\""
        ));
    }

    #[test]
    fn test_rewrite_grep_non_recursive_passthrough() {
        assert!(matches!(
            classify_command("grep pattern file.txt"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_rg() {
        assert!(matches!(
            classify_command("rg useState src/"),
            RewriteResult::Rewrite(s) if s == "ig \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("rg -i pattern"),
            RewriteResult::Rewrite(s) if s == "ig -i \"pattern\""
        ));
    }

    #[test]
    fn test_rewrite_rg_type_flag() {
        // Bug fix: -t flag value must be forwarded as --type to ig
        assert!(matches!(
            classify_command("rg -t ts pattern"),
            RewriteResult::Rewrite(s) if s == "ig --type ts \"pattern\""
        ));
        assert!(matches!(
            classify_command("rg -t rs useState src/"),
            RewriteResult::Rewrite(s) if s == "ig --type rs \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("rg -i -t ts pattern"),
            RewriteResult::Rewrite(s) if s == "ig -i --type ts \"pattern\""
        ));
    }

    #[test]
    fn test_rewrite_tree() {
        assert!(matches!(
            classify_command("tree"),
            RewriteResult::Rewrite(s) if s == "cat .ig/tree.txt 2>/dev/null || ig ls"
        ));
        // Bug fix: tree with flags must also be rewritten
        assert!(matches!(
            classify_command("tree -L 3 -I node_modules"),
            RewriteResult::Rewrite(s) if s == "cat .ig/tree.txt 2>/dev/null || ig ls"
        ));
        assert!(matches!(
            classify_command("tree -L 2"),
            RewriteResult::Rewrite(s) if s == "cat .ig/tree.txt 2>/dev/null || ig ls"
        ));
    }

    #[test]
    fn test_rewrite_find() {
        assert!(matches!(
            classify_command("find . -name \"*.ts\""),
            RewriteResult::Rewrite(s) if s == "ig files --glob \"*.ts\""
        ));
        // Bug fix: -type f must be allowed (ig only indexes files anyway)
        assert!(matches!(
            classify_command("find . -type f -name \"*.rs\""),
            RewriteResult::Rewrite(s) if s == "ig files --glob \"*.rs\""
        ));
        // -type d (directory) should not be rewritten
        assert!(matches!(
            classify_command("find . -type d -name src"),
            RewriteResult::Passthrough
        ));
        // Don't rewrite find with -exec
        assert!(matches!(
            classify_command("find . -name \"*.ts\" -exec rm {} ;"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_ls() {
        assert!(matches!(
            classify_command("ls"),
            RewriteResult::Rewrite(s) if s == "ig ls"
        ));
        assert!(matches!(
            classify_command("ls src/"),
            RewriteResult::Rewrite(s) if s == "ig ls src/"
        ));
        assert!(matches!(
            classify_command("ls -la src/"),
            RewriteResult::Rewrite(s) if s == "ig ls src/"
        ));
    }

    #[test]
    fn test_no_rewrite_pipes() {
        assert!(matches!(
            classify_command("echo hello | grep hello"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            classify_command("cat file && echo done"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_no_rewrite_empty() {
        assert!(matches!(classify_command(""), RewriteResult::Passthrough));
    }

    // --- Deny tests ---

    #[test]
    fn test_deny_git_reset_hard() {
        assert!(matches!(
            classify_command("git reset --hard"),
            RewriteResult::Deny(_)
        ));
        assert!(matches!(
            classify_command("git reset --hard HEAD~1"),
            RewriteResult::Deny(_)
        ));
    }

    #[test]
    fn test_deny_git_clean() {
        assert!(matches!(
            classify_command("git clean -f"),
            RewriteResult::Deny(_)
        ));
        assert!(matches!(
            classify_command("git clean -fd"),
            RewriteResult::Deny(_)
        ));
    }

    #[test]
    fn test_deny_rm_rf() {
        assert!(matches!(
            classify_command("rm -rf /"),
            RewriteResult::Deny(_)
        ));
        assert!(matches!(
            classify_command("rm -rf ."),
            RewriteResult::Deny(_)
        ));
        assert!(matches!(
            classify_command("rm -rf ~"),
            RewriteResult::Deny(_)
        ));
    }

    // --- Ask tests ---

    #[test]
    fn test_ask_git_push_force() {
        assert!(matches!(
            classify_command("git push --force"),
            RewriteResult::Ask(_)
        ));
        assert!(matches!(
            classify_command("git push -f"),
            RewriteResult::Ask(_)
        ));
        assert!(matches!(
            classify_command("git push --force-with-lease"),
            RewriteResult::Ask(_)
        ));
    }

    // --- Passthrough tests for safe git commands ---

    #[test]
    fn test_git_rewrite() {
        // Read-only git commands are rewritten to ig git
        assert!(matches!(
            classify_command("git status"),
            RewriteResult::Rewrite(_)
        ));
        assert!(matches!(
            classify_command("git log"),
            RewriteResult::Rewrite(_)
        ));
        assert!(matches!(
            classify_command("git diff"),
            RewriteResult::Rewrite(_)
        ));
        assert!(matches!(
            classify_command("git show HEAD"),
            RewriteResult::Rewrite(_)
        ));
    }

    #[test]
    fn test_passthrough_write_git() {
        // Write/destructive git commands pass through (not rewritten)
        assert!(matches!(
            classify_command("git commit -m test"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            classify_command("git checkout main"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            classify_command("cargo test"),
            RewriteResult::Passthrough
        ));
    }
}
