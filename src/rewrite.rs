/// Command rewriting engine — intercepts shell commands and maps them to ig equivalents.
/// Used by the PreToolUse hook to transparently redirect cat/grep/ls/tree/find to ig.
///
/// Exit codes (same protocol as RTK):
///   0 + stdout  → rewrite found, auto-allow
///   1           → no rewrite, passthrough

use std::process;

pub fn run_rewrite(command: &str) {
    match rewrite_command(command) {
        Some(rewritten) => {
            print!("{}", rewritten);
            process::exit(0);
        }
        None => {
            process::exit(1);
        }
    }
}

fn rewrite_command(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();

    // Skip empty or compound commands (pipes, &&, ||, ;)
    if cmd.is_empty() || cmd.contains('|') || cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') {
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
        *p == "-r" || *p == "-R" || *p == "--recursive"
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
    let case_flag = if parts.iter().any(|p| {
        *p == "-i" || (p.starts_with('-') && !p.starts_with("--") && p.contains('i'))
    }) { " -i" } else { "" };

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
    if parts.iter().any(|p| *p == "-exec" || *p == "-delete" || *p == "-print0") {
        return None;
    }

    // Allow -type f (file-only filter — always safe to ignore since ig only indexes files)
    // Reject other -type values (d, l, etc.)
    let mut i = 1;
    while i < parts.len() {
        if parts[i] == "-type" {
            if let Some(val) = parts.get(i + 1) {
                if *val != "f" {
                    return None;
                }
                i += 2;
                continue;
            }
        }
        i += 1;
    }

    Some(format!("ig files --glob {}", pattern))
}

/// ls [dir] → ig ls [dir]
fn rewrite_ls(parts: &[&str]) -> Option<String> {
    // Collect non-flag args
    let args: Vec<&str> = parts.iter().skip(1).filter(|p| !p.starts_with('-')).copied().collect();

    match args.len() {
        0 => Some("ig ls".to_string()),
        1 => Some(format!("ig ls {}", args[0])),
        _ => None, // Multiple paths — don't rewrite
    }
}

/// Simple shell-like splitting (handles quotes minimally)
fn shell_split(cmd: &str) -> Vec<&str> {
    cmd.split_whitespace().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_cat() {
        assert_eq!(rewrite_command("cat src/main.rs"), Some("ig read src/main.rs".into()));
        assert_eq!(rewrite_command("cat -n src/main.rs"), None); // has flags
    }

    #[test]
    fn test_rewrite_head() {
        assert_eq!(rewrite_command("head src/main.rs"), Some("ig read src/main.rs".into()));
        assert_eq!(rewrite_command("head -50 src/main.rs"), Some("ig read src/main.rs".into()));
    }

    #[test]
    fn test_rewrite_tail() {
        assert_eq!(rewrite_command("tail src/main.rs"), Some("ig read src/main.rs".into()));
        assert_eq!(rewrite_command("tail -20 src/main.rs"), Some("ig read src/main.rs".into()));
    }

    #[test]
    fn test_rewrite_grep_recursive() {
        assert_eq!(
            rewrite_command("grep -rn useState src/"),
            Some("ig \"useState\" src/".into())
        );
        assert_eq!(
            rewrite_command("grep -ri pattern ."),
            Some("ig -i \"pattern\"".into())
        );
    }

    #[test]
    fn test_rewrite_grep_non_recursive_passthrough() {
        assert_eq!(rewrite_command("grep pattern file.txt"), None);
    }

    #[test]
    fn test_rewrite_rg() {
        assert_eq!(rewrite_command("rg useState src/"), Some("ig \"useState\" src/".into()));
        assert_eq!(rewrite_command("rg -i pattern"), Some("ig -i \"pattern\"".into()));
    }

    #[test]
    fn test_rewrite_rg_type_flag() {
        // Bug fix: -t flag value must be forwarded as --type to ig
        assert_eq!(
            rewrite_command("rg -t ts pattern"),
            Some("ig --type ts \"pattern\"".into())
        );
        assert_eq!(
            rewrite_command("rg -t rs useState src/"),
            Some("ig --type rs \"useState\" src/".into())
        );
        assert_eq!(
            rewrite_command("rg -i -t ts pattern"),
            Some("ig -i --type ts \"pattern\"".into())
        );
    }

    #[test]
    fn test_rewrite_tree() {
        assert_eq!(
            rewrite_command("tree"),
            Some("cat .ig/tree.txt 2>/dev/null || ig ls".into())
        );
        // Bug fix: tree with flags must also be rewritten
        assert_eq!(
            rewrite_command("tree -L 3 -I node_modules"),
            Some("cat .ig/tree.txt 2>/dev/null || ig ls".into())
        );
        assert_eq!(
            rewrite_command("tree -L 2"),
            Some("cat .ig/tree.txt 2>/dev/null || ig ls".into())
        );
    }

    #[test]
    fn test_rewrite_find() {
        assert_eq!(
            rewrite_command("find . -name \"*.ts\""),
            Some("ig files --glob \"*.ts\"".into())
        );
        // Bug fix: -type f must be allowed (ig only indexes files anyway)
        assert_eq!(
            rewrite_command("find . -type f -name \"*.rs\""),
            Some("ig files --glob \"*.rs\"".into())
        );
        // -type d (directory) should not be rewritten
        assert_eq!(rewrite_command("find . -type d -name src"), None);
        // Don't rewrite find with -exec
        assert_eq!(rewrite_command("find . -name \"*.ts\" -exec rm {} ;"), None);
    }

    #[test]
    fn test_rewrite_ls() {
        assert_eq!(rewrite_command("ls"), Some("ig ls".into()));
        assert_eq!(rewrite_command("ls src/"), Some("ig ls src/".into()));
        assert_eq!(rewrite_command("ls -la src/"), Some("ig ls src/".into()));
    }

    #[test]
    fn test_no_rewrite_git() {
        assert_eq!(rewrite_command("git status"), None);
        assert_eq!(rewrite_command("cargo test"), None);
    }

    #[test]
    fn test_no_rewrite_pipes() {
        assert_eq!(rewrite_command("echo hello | grep hello"), None);
        assert_eq!(rewrite_command("cat file && echo done"), None);
    }

    #[test]
    fn test_no_rewrite_empty() {
        assert_eq!(rewrite_command(""), None);
    }
}
