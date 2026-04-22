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
    let parts = shell_split(cmd);
    // git reset --hard [ref]
    parts.len() >= 3
        && parts[0] == "git"
        && parts[1] == "reset"
        && parts.iter().any(|p| p == "--hard")
}

fn is_deny_git_clean(cmd: &str) -> bool {
    let parts = shell_split(cmd);
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
    let parts = shell_split(cmd);
    if parts.is_empty() || parts[0] != "rm" {
        return false;
    }
    let has_recursive = parts.iter().any(|p| {
        p == "-r"
            || p == "-R"
            || p == "--recursive"
            || (p.starts_with('-') && !p.starts_with("--") && (p.contains('r') || p.contains('R')))
    });
    let has_force = parts.iter().any(|p| {
        p == "-f"
            || p == "--force"
            || (p.starts_with('-') && !p.starts_with("--") && p.contains('f'))
    });
    if !has_recursive || !has_force {
        return false;
    }
    // Check for dangerous targets — strip trailing '/' before comparison (Fix R3)
    let dangerous_targets = ["/", ".", "~"];
    parts
        .iter()
        .filter(|p| !p.starts_with('-'))
        .skip(1) // skip "rm"
        .any(|p| {
            // Strip trailing slashes; if that empties the string it was "/"
            let normalized = p.trim_end_matches('/');
            let normalized = if normalized.is_empty() {
                "/"
            } else {
                normalized
            };
            dangerous_targets.contains(&normalized)
        })
}

fn is_ask_git_push_force(cmd: &str) -> bool {
    let parts = shell_split(cmd);
    if parts.len() < 2 || parts[0] != "git" || parts[1] != "push" {
        return false;
    }
    parts
        .iter()
        .skip(2)
        .any(|p| p == "--force" || p == "-f" || p == "--force-with-lease")
}

fn try_rewrite(cmd: &str) -> Option<String> {
    if cmd.trim().is_empty() {
        return None;
    }

    // Split on top-level shell operators (pipes, &&, ||, ;), respecting quotes.
    // For pipes (`|`), only the first segment is rewritten (stream semantics must
    // be preserved for downstream filters). For `&&`/`||`/`;`, each segment is
    // rewritten independently since they are sequenced commands.
    let parts = split_top_level(cmd);
    if parts.is_empty() {
        return None;
    }

    let mut any_rewrite = false;
    let mut out = String::new();
    let mut seen_pipe = false;

    for (i, seg) in parts.iter().enumerate() {
        let segment = &seg.text;
        let op = seg.op.as_deref(); // trailing operator, if any

        // Once we cross a pipe, downstream segments keep their original form
        // (stdin-based filters like `head -20`, `wc -l`, `grep x` should not be
        // touched — they operate on the upstream stdout).
        let skip_segment = seen_pipe;

        let rewritten_seg = if skip_segment {
            segment.to_string()
        } else {
            match try_rewrite_segment(segment) {
                Some(r) => {
                    any_rewrite = true;
                    r
                }
                None => segment.to_string(),
            }
        };

        out.push_str(&rewritten_seg);
        if let Some(o) = op {
            out.push_str(o);
        }
        if op == Some(" | ") {
            seen_pipe = true;
        }
        // Guard: if last part without op, no trailing separator needed
        let _ = i;
    }

    if any_rewrite { Some(out) } else { None }
}

/// Rewrite a single command segment (no pipes / operators inside).
/// Handles env prefix stripping (`sudo`, `env`, `VAR=val`) and absolute
/// binary path normalization (`/usr/bin/grep` → `grep`) before dispatching.
fn try_rewrite_segment(segment: &str) -> Option<String> {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (env_prefix, rest) = split_env_prefix(trimmed);
    let parts_raw = shell_split(rest);
    if parts_raw.is_empty() {
        return None;
    }

    // Normalize absolute paths: `/usr/bin/grep` → `grep`. Keep the rest intact.
    let mut parts: Vec<String> = parts_raw;
    parts[0] = strip_absolute_path(&parts[0]).to_string();

    let bin = parts[0].as_str();
    let rewritten = match bin {
        "cat" => rewrite_cat(&parts),
        "head" => rewrite_head(&parts),
        "tail" => rewrite_tail(&parts),
        "grep" | "egrep" | "fgrep" => rewrite_grep(&parts),
        "rg" => rewrite_rg(&parts),
        "tree" => rewrite_tree(&parts),
        "find" => rewrite_find(&parts),
        "ls" => rewrite_ls(&parts),
        "git" => rewrite_git(&parts),
        // Commands routed through `ig run` filter engine
        "cargo" => rewrite_via_run(&parts),
        "docker" => rewrite_docker(&parts),
        "kubectl" => rewrite_via_run(&parts),
        "pytest" | "ruff" | "mypy" => rewrite_via_run(&parts),
        "eslint" | "biome" | "prettier" | "tsc" => rewrite_via_run(&parts),
        "vitest" | "jest" | "playwright" => rewrite_via_run(&parts),
        "go" => rewrite_via_run(&parts),
        "golangci-lint" => rewrite_via_run(&parts),
        "dotnet" => rewrite_via_run(&parts),
        "rspec" | "rubocop" | "rake" => rewrite_via_run(&parts),
        "gh" => rewrite_via_run(&parts),
        "aws" | "gcloud" => rewrite_via_run(&parts),
        "psql" => rewrite_via_run(&parts),
        "pnpm" => rewrite_via_run(&parts),
        "npm" => rewrite_npm(&parts),
        "npx" => rewrite_npx(&parts),
        "wc" => rewrite_via_run(&parts),
        "curl" | "wget" => rewrite_via_run(&parts),
        "rsync" | "ping" => rewrite_via_run(&parts),
        "make" | "mvn" | "bundle" | "swift" | "mix" => rewrite_via_run(&parts),
        "shellcheck" | "yamllint" | "markdownlint" | "hadolint" => rewrite_via_run(&parts),
        "pre-commit" | "trunk" => rewrite_via_run(&parts),
        "helm" | "terraform" | "tofu" => rewrite_via_run(&parts),
        "ansible-playbook" | "systemctl" => rewrite_via_run(&parts),
        "pip" | "poetry" | "uv" | "composer" | "brew" | "pio" => rewrite_via_run(&parts),
        "next" | "prisma" => rewrite_via_run(&parts),
        "df" | "du" | "ps" => rewrite_via_run(&parts),
        "diff" => rewrite_via_run(&parts),
        _ => None,
    }?;

    if env_prefix.is_empty() {
        Some(rewritten)
    } else {
        Some(format!("{}{}", env_prefix, rewritten))
    }
}

/// cat file → ig read --plain file (or -s for large source files)
fn rewrite_cat(parts: &[String]) -> Option<String> {
    // Only rewrite simple `cat file` (no flags like -n, -A, etc.)
    if parts.len() != 2 || parts[1].starts_with('-') {
        return None;
    }
    let file = &parts[1];
    if large_code_file(file) {
        Some(format!("ig read {} -s", file))
    } else {
        Some(format!("ig read --plain {}", file))
    }
}

/// Returns true when the file is a "large" source file worth signature-only output.
/// Heuristic: extension in the code-file set AND file size > 8 KB (≈ 300 lines of code).
fn large_code_file(path: &str) -> bool {
    const CODE_EXT: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "php", "java", "kt", "scala",
        "cpp", "cc", "c", "h", "hpp", "rb", "swift", "cs",
    ];
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !CODE_EXT.iter().any(|e| *e == ext) {
        return false;
    }
    std::fs::metadata(path)
        .map(|m| m.len() > 8_000)
        .unwrap_or(false)
}

/// head -N file → ig read file (first N lines shown by default)
fn rewrite_head(parts: &[String]) -> Option<String> {
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
fn rewrite_tail(parts: &[String]) -> Option<String> {
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
fn rewrite_grep(parts: &[String]) -> Option<String> {
    // Only intercept recursive grep (code search)
    let has_recursive = parts.iter().any(|p| {
        p == "-r"
            || p == "-R"
            || p == "--recursive"
            || (p.starts_with('-') && !p.starts_with("--") && (p.contains('r') || p.contains('R')))
    });

    if !has_recursive {
        return None;
    }

    // Extract pattern and path
    let mut pattern: Option<&str> = None;
    let mut path: Option<&str> = None;
    let mut skip_next = false;
    let mut next_is_pattern = false;

    for part in parts.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if next_is_pattern {
            pattern = Some(part.as_str());
            next_is_pattern = false;
            continue;
        }
        if part.starts_with('-') {
            if part == "-e" {
                // Fix R2: next token is the explicit pattern
                next_is_pattern = true;
            } else if part == "--include" || part == "--exclude" {
                skip_next = true;
            }
            continue;
        }
        if pattern.is_none() {
            pattern = Some(part.as_str());
        } else if path.is_none() {
            path = Some(part.as_str());
        }
    }

    let pattern = pattern?;
    let case_flag = if parts
        .iter()
        .any(|p| p == "-i" || (p.starts_with('-') && !p.starts_with("--") && p.contains('i')))
    {
        " -i"
    } else {
        ""
    };

    match path {
        Some(p) if p != "." => Some(format!(
            "IG_COMPACT=1 ig{} \"{}\" {}",
            case_flag, pattern, p
        )),
        _ => Some(format!("IG_COMPACT=1 ig{} \"{}\"", case_flag, pattern)),
    }
}

/// rg pattern [path] → ig "pattern" [path]
fn rewrite_rg(parts: &[String]) -> Option<String> {
    let mut pattern: Option<&str> = None;
    let mut path: Option<&str> = None;
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
            type_filter = Some(part.as_str());
            next_is_type = false;
            continue;
        }
        if part == "-i" || part == "--ignore-case" {
            case_flag = " -i";
            continue;
        }
        if part.starts_with('-') {
            if part == "-t" || part == "--type" {
                next_is_type = true;
            } else if part == "-g" || part == "--glob" {
                skip_next = true;
            }
            continue;
        }
        if pattern.is_none() {
            pattern = Some(part.as_str());
        } else if path.is_none() {
            path = Some(part.as_str());
        }
    }

    let pattern = pattern?;
    let type_arg = match type_filter {
        Some(t) => format!(" --type {}", t),
        None => String::new(),
    };
    match path {
        Some(p) => Some(format!(
            "IG_COMPACT=1 ig{}{} \"{}\" {}",
            case_flag, type_arg, pattern, p
        )),
        None => Some(format!(
            "IG_COMPACT=1 ig{}{} \"{}\"",
            case_flag, type_arg, pattern
        )),
    }
}

/// tree → cat .ig/tree.txt (if exists) or ig ls
fn rewrite_tree(_parts: &[String]) -> Option<String> {
    // Always rewrite tree (with or without flags like -L N -I pattern)
    Some("cat .ig/tree.txt 2>/dev/null || ig ls".to_string())
}

/// find . -name "*.ts" → ig files --glob "*.ts"
fn rewrite_find(parts: &[String]) -> Option<String> {
    // Only rewrite find with -name pattern
    let name_idx = parts.iter().position(|p| p == "-name" || p == "-iname")?;
    let pattern = parts.get(name_idx + 1)?;

    // Skip if there are destructive or complex action flags
    if parts
        .iter()
        .any(|p| p == "-exec" || p == "-delete" || p == "-print0")
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
            if val != "f" {
                return None;
            }
            i += 2;
            continue;
        }
        i += 1;
    }

    // Fix R4: quote the glob pattern in output
    Some(format!("ig files --glob \"{}\"", pattern))
}

/// ls [dir] → ig ls [dir]
/// Only rewrite when there's a path arg OR an informative flag (-l, -a, -R).
/// Bare `ls` on cwd already produces a terse listing; rewriting adds noise.
fn rewrite_ls(parts: &[String]) -> Option<String> {
    let has_informative_flag = parts
        .iter()
        .skip(1)
        .any(|p| p.starts_with('-') && (p.contains('l') || p.contains('a') || p.contains('R')));

    let args: Vec<&str> = parts
        .iter()
        .skip(1)
        .filter(|p| !p.starts_with('-'))
        .map(|p| p.as_str())
        .collect();

    // Skip rewriting if the single path contains shell glob metacharacters:
    // the glob is expanded by the shell at exec time, producing multiple
    // arguments that `ig ls` cannot accept.
    let contains_glob = |s: &str| s.contains('*') || s.contains('?') || s.contains('[');

    match args.len() {
        0 if !has_informative_flag => None, // bare `ls` — pass through
        0 => Some("ig ls".to_string()),
        1 if contains_glob(args[0]) => None, // glob path — let real ls handle it
        // `ls <path>` without flags: raw output already compact — passthrough.
        // Only rewrite when an informative flag is present (-l, -a, -R).
        1 if !has_informative_flag => None,
        1 => Some(format!("ig ls {}", args[0])),
        _ => None,
    }
}

/// git status/log/diff/branch/show → ig git <subcmd> [args]
/// Destructive commands (push, reset, checkout, clean, rebase, merge, commit) are NOT rewritten.
///
/// Git global options (`-C <path>`, `-c <k=v>`, `--git-dir <dir>`, `--work-tree <dir>`,
/// `--no-pager`, `--no-optional-locks`, `--bare`, `--literal-pathspecs`) before the
/// subcommand are stripped — they affect execution context but not the classification.
fn rewrite_git(parts: &[String]) -> Option<String> {
    if parts.len() < 2 {
        return None;
    }

    // Skip global options to locate the actual subcommand.
    let mut i = 1;
    while i < parts.len() {
        let p = parts[i].as_str();
        match p {
            "-C" | "-c" => {
                // Takes a value argument.
                i += 2;
            }
            "--git-dir" | "--work-tree" => {
                // May be --git-dir=/path OR --git-dir /path
                i += 2;
            }
            "--no-pager" | "--no-optional-locks" | "--bare" | "--literal-pathspecs" => {
                i += 1;
            }
            s if s.starts_with("--git-dir=") || s.starts_with("--work-tree=") => {
                i += 1;
            }
            _ => break,
        }
    }

    if i >= parts.len() {
        return None;
    }

    let subcmd = parts[i].as_str();
    // Only rewrite read-only git subcommands
    match subcmd {
        "status" | "log" | "diff" | "branch" | "show" => {
            let args = parts[i + 1..].join(" ");
            if args.is_empty() {
                Some(format!("ig git {}", subcmd))
            } else {
                Some(format!("ig git {} {}", subcmd, args))
            }
        }
        _ => None, // Don't rewrite destructive/write commands
    }
}

/// Generic rewrite: command → ig run command (for TOML-filtered commands)
fn rewrite_via_run(parts: &[String]) -> Option<String> {
    let cmd = parts.join(" ");
    Some(format!("ig run {}", cmd))
}

/// docker subcmd → ig docker subcmd (only for non-interactive commands)
fn rewrite_docker(parts: &[String]) -> Option<String> {
    if parts.len() < 2 {
        return None;
    }
    // Don't rewrite interactive docker commands
    match parts[1].as_str() {
        "ps" | "images" | "logs" | "build" | "compose" | "inspect" | "stats" | "top" => {
            let args = parts[1..].join(" ");
            Some(format!("ig docker {}", args))
        }
        _ => None, // exec, run, etc. are interactive — passthrough
    }
}

/// npm run/exec → ig run npm run/exec (skip npm install/ci)
fn rewrite_npm(parts: &[String]) -> Option<String> {
    if parts.len() < 2 {
        return None;
    }
    match parts[1].as_str() {
        "run" | "exec" | "test" => {
            let cmd = parts.join(" ");
            Some(format!("ig run {}", cmd))
        }
        _ => None, // Don't rewrite npm install, npm ci, etc.
    }
}

/// npx tool → ig run npx tool (route through filter engine)
fn rewrite_npx(parts: &[String]) -> Option<String> {
    if parts.len() < 2 {
        return None;
    }
    let cmd = parts.join(" ");
    Some(format!("ig run {}", cmd))
}

/// A segment of a shell command, along with the operator that follows it (if any).
#[derive(Debug, Clone)]
struct Segment {
    text: String,
    /// Separator between this segment and the next: " | ", " && ", " || ", " ; ".
    /// None for the last segment.
    op: Option<String>,
}

/// Split a command on top-level shell operators (`|`, `||`, `&&`, `;`),
/// respecting single and double quotes. Does not split on `|` when it is `||`
/// (logical OR). Each returned Segment keeps its original trailing operator so
/// the caller can recompose the command after rewriting individual segments.
fn split_top_level(cmd: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut current = String::new();
    let bytes: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        let c = bytes[i];

        if !in_single && !in_double {
            // Check for multi-char operators first
            if c == '&' && bytes.get(i + 1) == Some(&'&') {
                segments.push(Segment {
                    text: current.trim().to_string(),
                    op: Some(" && ".to_string()),
                });
                current.clear();
                i += 2;
                continue;
            }
            if c == '|' && bytes.get(i + 1) == Some(&'|') {
                segments.push(Segment {
                    text: current.trim().to_string(),
                    op: Some(" || ".to_string()),
                });
                current.clear();
                i += 2;
                continue;
            }
            if c == '|' {
                segments.push(Segment {
                    text: current.trim().to_string(),
                    op: Some(" | ".to_string()),
                });
                current.clear();
                i += 1;
                continue;
            }
            if c == ';' {
                segments.push(Segment {
                    text: current.trim().to_string(),
                    op: Some("; ".to_string()),
                });
                current.clear();
                i += 1;
                continue;
            }
        }

        // Quote state tracking
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c == '\\' && in_double {
            // Preserve escape + next char
            current.push(c);
            if let Some(&next) = bytes.get(i + 1) {
                current.push(next);
                i += 2;
                continue;
            }
        }

        current.push(c);
        i += 1;
    }

    let last = current.trim().to_string();
    if !last.is_empty() {
        segments.push(Segment {
            text: last,
            op: None,
        });
    }

    segments
}

/// Strip leading env/sudo prefixes from a segment.
/// Returns `(prefix_with_trailing_space, remainder)`.
///
/// Handles repeated prefixes like `sudo RUST_LOG=debug cargo test`.
fn split_env_prefix(segment: &str) -> (String, &str) {
    let mut prefix = String::new();
    let mut rest = segment;

    loop {
        let trimmed = rest.trim_start();
        // `sudo` / `env` tokens
        if let Some(after) = trimmed.strip_prefix("sudo ") {
            prefix.push_str("sudo ");
            rest = after;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix("env ") {
            prefix.push_str("env ");
            rest = after;
            continue;
        }
        // `VAR=value` assignment — token ends at first unquoted space
        if let Some((var, after)) = try_take_env_assignment(trimmed) {
            prefix.push_str(var);
            prefix.push(' ');
            rest = after;
            continue;
        }
        break;
    }

    (prefix, rest.trim_start())
}

/// If the string starts with `VAR=value` (value may be quoted), return
/// (token, remainder_without_leading_space).
fn try_take_env_assignment(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    // Find the '=' sign, making sure what precedes is [A-Z_][A-Z0-9_]*.
    let eq = s.find('=')?;
    if eq == 0 {
        return None;
    }
    let var = &s[..eq];
    let first = var.chars().next()?;
    if !(first == '_' || first.is_ascii_uppercase()) {
        return None;
    }
    if !var
        .chars()
        .all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }

    // Find the end of the value: first unquoted whitespace after eq.
    let mut i = eq + 1;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c.is_ascii_whitespace() && !in_single && !in_double {
            break;
        }
        i += 1;
    }

    let token = &s[..i];
    // Skip the whitespace separating the assignment from the command.
    let mut rest = &s[i..];
    rest = rest.trim_start();
    Some((token, rest))
}

/// Normalize absolute binary paths: `/usr/bin/grep` → `grep`.
/// Leaves relative paths and simple names unchanged.
fn strip_absolute_path(bin: &str) -> &str {
    if bin.starts_with('/') {
        bin.rsplit('/').next().unwrap_or(bin)
    } else {
        bin
    }
}

/// Quote-aware shell tokenizer (Fix R1).
///
/// Handles double and single quotes, stripping them from the resulting tokens.
/// Supports escaped characters within double-quoted strings.
fn shell_split(cmd: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_double = false;
    let mut in_single = false;
    let mut in_token = false;

    while let Some(c) = chars.next() {
        match c {
            '"' if !in_single => {
                in_double = !in_double;
                in_token = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                in_token = true;
            }
            '\\' if in_double => {
                // Escape sequence inside double quotes
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                in_token = true;
            }
            ' ' | '\t' if !in_double && !in_single => {
                if in_token {
                    tokens.push(current.clone());
                    current.clear();
                    in_token = false;
                }
            }
            _ => {
                current.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Rewrite tests (via classify_command) ---

    #[test]
    fn test_rewrite_cat() {
        // A non-existent / small file routes to --plain.
        assert!(matches!(
            classify_command("cat /tmp/__ig_nonexistent_file__"),
            RewriteResult::Rewrite(s) if s == "ig read --plain /tmp/__ig_nonexistent_file__"
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
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("grep -ri pattern ."),
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig -i \"pattern\""
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
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("rg -i pattern"),
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig -i \"pattern\""
        ));
    }

    #[test]
    fn test_rewrite_rg_type_flag() {
        // Bug fix: -t flag value must be forwarded as --type to ig
        assert!(matches!(
            classify_command("rg -t ts pattern"),
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig --type ts \"pattern\""
        ));
        assert!(matches!(
            classify_command("rg -t rs useState src/"),
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig --type rs \"useState\" src/"
        ));
        assert!(matches!(
            classify_command("rg -i -t ts pattern"),
            RewriteResult::Rewrite(s) if s == "IG_COMPACT=1 ig -i --type ts \"pattern\""
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
            classify_command(r#"find . -name "*.ts""#),
            RewriteResult::Rewrite(s) if s == r#"ig files --glob "*.ts""#
        ));
        // Bug fix: -type f must be allowed (ig only indexes files anyway)
        assert!(matches!(
            classify_command(r#"find . -type f -name "*.rs""#),
            RewriteResult::Rewrite(s) if s == r#"ig files --glob "*.rs""#
        ));
        // -type d (directory) should not be rewritten
        assert!(matches!(
            classify_command("find . -type d -name src"),
            RewriteResult::Passthrough
        ));
        // Don't rewrite find with -exec
        assert!(matches!(
            classify_command(r#"find . -name "*.ts" -exec rm {} ;"#),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_ls() {
        // Bare `ls` and `ls <path>` are passthrough — raw output is already terse
        // and rewriting adds formatting overhead for small directories.
        assert!(matches!(classify_command("ls"), RewriteResult::Passthrough));
        assert!(matches!(
            classify_command("ls src/"),
            RewriteResult::Passthrough
        ));
        // Informative flags (-l, -a, -R) trigger rewrite
        assert!(matches!(
            classify_command("ls -la"),
            RewriteResult::Rewrite(s) if s == "ig ls"
        ));
        assert!(matches!(
            classify_command("ls -la src/"),
            RewriteResult::Rewrite(s) if s == "ig ls src/"
        ));
    }

    #[test]
    fn test_pipes_and_compounds() {
        // Downstream segments after a pipe keep their original form (stdin
        // filters like grep-on-stdin must not be touched).
        assert!(matches!(
            classify_command("echo hello | grep hello"),
            RewriteResult::Passthrough
        ));
        // Compound via `&&` rewrites each segment independently.
        assert!(matches!(
            classify_command("cat file && echo done"),
            RewriteResult::Rewrite(s) if s == "ig read --plain file && echo done"
        ));
    }

    // --- Phase A: pipelines, env prefix, absolute paths ---

    #[test]
    fn test_rewrite_rg_with_pipe() {
        // `rg pat path | head -20` should rewrite the rg segment only;
        // `head -20` stays since it's a stdin-based filter.
        assert!(matches!(
            classify_command("rg useState src | head -20"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "useState" src | head -20"#
        ));
    }

    #[test]
    fn test_rewrite_grep_with_pipe() {
        assert!(matches!(
            classify_command("grep -rn foo src | wc -l"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "foo" src | wc -l"#
        ));
    }

    #[test]
    fn test_rewrite_env_prefix() {
        // `ENV=1 rg pat` → env preserved, bin rewritten
        assert!(matches!(
            classify_command("RUST_LOG=debug rg useState src"),
            RewriteResult::Rewrite(s) if s == r#"RUST_LOG=debug IG_COMPACT=1 ig "useState" src"#
        ));
    }

    #[test]
    fn test_rewrite_multiple_env_prefix() {
        assert!(matches!(
            classify_command("A=1 B=2 rg pat src"),
            RewriteResult::Rewrite(s) if s == r#"A=1 B=2 IG_COMPACT=1 ig "pat" src"#
        ));
    }

    #[test]
    fn test_rewrite_absolute_path_bin() {
        // `/usr/bin/grep -r pat src` → bin normalized before matching
        assert!(matches!(
            classify_command("/usr/bin/grep -rn useState src/"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "useState" src/"#
        ));
        assert!(matches!(
            classify_command("/opt/homebrew/bin/rg pat src"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "pat" src"#
        ));
    }

    #[test]
    fn test_rewrite_compound_semicolon() {
        // `;` sequences each segment independently
        assert!(matches!(
            classify_command("rg foo src ; ls -la src"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "foo" src; ig ls src"#
        ));
    }

    #[test]
    fn test_rewrite_compound_or() {
        assert!(matches!(
            classify_command("cargo test || echo fail"),
            RewriteResult::Rewrite(s) if s == "ig run cargo test || echo fail"
        ));
    }

    #[test]
    fn test_double_pipe_not_split_like_single() {
        // `foo || bar`: logical OR, each segment rewritten independently
        assert!(matches!(
            classify_command("ls -la src || ls -la ."),
            RewriteResult::Rewrite(s) if s == "ig ls src || ig ls ."
        ));
    }

    #[test]
    fn test_quotes_with_pipe_char_inside() {
        // Pipe character inside quotes must not split
        let segs = split_top_level(r#"rg "a|b" src"#);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, r#"rg "a|b" src"#);
    }

    #[test]
    fn test_strip_absolute_path_fn() {
        assert_eq!(strip_absolute_path("/usr/bin/grep"), "grep");
        assert_eq!(strip_absolute_path("grep"), "grep");
        assert_eq!(strip_absolute_path("./local/bin"), "./local/bin");
    }

    #[test]
    fn test_rewrite_git_global_opts() {
        assert!(matches!(
            classify_command("git -C /tmp/repo status"),
            RewriteResult::Rewrite(s) if s == "ig git status"
        ));
        assert!(matches!(
            classify_command("git -C /tmp/repo log --oneline"),
            RewriteResult::Rewrite(s) if s == "ig git log --oneline"
        ));
        assert!(matches!(
            classify_command("git --no-pager diff"),
            RewriteResult::Rewrite(s) if s == "ig git diff"
        ));
        assert!(matches!(
            classify_command("git --git-dir=/tmp/.git status"),
            RewriteResult::Rewrite(s) if s == "ig git status"
        ));
        // Global opts before a destructive subcommand stay passthrough
        assert!(matches!(
            classify_command("git -C /tmp/repo commit -m test"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_ls_glob_passthrough() {
        // Glob in path is expanded by shell at exec time → `ig ls` would get
        // multiple args and fail. Passthrough to real ls.
        assert!(matches!(
            classify_command("ls /tmp/session-*.jsonl"),
            RewriteResult::Passthrough
        ));
        assert!(matches!(
            classify_command("ls src/*.rs"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_split_env_prefix_fn() {
        let (p, r) = split_env_prefix("RUST_LOG=debug cargo test");
        assert_eq!(p, "RUST_LOG=debug ");
        assert_eq!(r, "cargo test");

        let (p, r) = split_env_prefix("sudo A=1 grep -r foo src");
        assert_eq!(p, "sudo A=1 ");
        assert_eq!(r, "grep -r foo src");

        let (p, r) = split_env_prefix("just a command");
        assert_eq!(p, "");
        assert_eq!(r, "just a command");
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
        // cargo test is now rewritten to ig run cargo test
        assert!(matches!(
            classify_command("cargo test"),
            RewriteResult::Rewrite(_)
        ));
    }

    // --- New tests for fixes R1/R2/R3 ---

    #[test]
    fn test_shell_split_quotes() {
        let parts = shell_split(r#"grep -r "hello world" src/"#);
        assert_eq!(parts, vec!["grep", "-r", "hello world", "src/"]);
    }

    #[test]
    fn test_shell_split_single_quotes() {
        let parts = shell_split("cat 'my file.rs'");
        assert_eq!(parts, vec!["cat", "my file.rs"]);
    }

    #[test]
    fn test_rewrite_grep_e_flag() {
        assert!(matches!(
            classify_command("grep -r -e pattern src/"),
            RewriteResult::Rewrite(s) if s == r#"IG_COMPACT=1 ig "pattern" src/"#
        ));
    }

    #[test]
    fn test_deny_rm_rf_dot_slash() {
        assert!(matches!(
            classify_command("rm -rf ./"),
            RewriteResult::Deny(_)
        ));
    }

    #[test]
    fn test_deny_rm_rf_tilde_slash() {
        assert!(matches!(
            classify_command("rm -rf ~/"),
            RewriteResult::Deny(_)
        ));
    }

    // --- New rewrite rules for TOML-filtered commands ---

    #[test]
    fn test_rewrite_cargo() {
        assert!(matches!(
            classify_command("cargo test"),
            RewriteResult::Rewrite(s) if s == "ig run cargo test"
        ));
        assert!(matches!(
            classify_command("cargo build --release"),
            RewriteResult::Rewrite(s) if s == "ig run cargo build --release"
        ));
        assert!(matches!(
            classify_command("cargo clippy"),
            RewriteResult::Rewrite(s) if s == "ig run cargo clippy"
        ));
    }

    #[test]
    fn test_rewrite_docker() {
        assert!(matches!(
            classify_command("docker ps"),
            RewriteResult::Rewrite(s) if s == "ig docker ps"
        ));
        assert!(matches!(
            classify_command("docker logs -f app"),
            RewriteResult::Rewrite(s) if s == "ig docker logs -f app"
        ));
    }

    #[test]
    fn test_rewrite_pytest() {
        assert!(matches!(
            classify_command("pytest -v tests/"),
            RewriteResult::Rewrite(s) if s == "ig run pytest -v tests/"
        ));
    }

    #[test]
    fn test_rewrite_npm_selective() {
        assert!(matches!(
            classify_command("npm run build"),
            RewriteResult::Rewrite(s) if s == "ig run npm run build"
        ));
        assert!(matches!(
            classify_command("npm test"),
            RewriteResult::Rewrite(s) if s == "ig run npm test"
        ));
        // npm install should NOT be rewritten
        assert!(matches!(
            classify_command("npm install"),
            RewriteResult::Passthrough
        ));
    }

    #[test]
    fn test_rewrite_kubectl() {
        assert!(matches!(
            classify_command("kubectl get pods"),
            RewriteResult::Rewrite(s) if s == "ig run kubectl get pods"
        ));
    }

    #[test]
    fn test_rewrite_gh() {
        assert!(matches!(
            classify_command("gh pr list"),
            RewriteResult::Rewrite(s) if s == "ig run gh pr list"
        ));
    }
}
