//! Git command proxy — token-compressed output for AI agents.
//! Replaces verbose git output with compact summaries.

use std::process::Command;

use crate::tracking;

/// Run a git subcommand with token-compressed output.
pub fn run_git(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: ig git <subcommand> [args...]");
        std::process::exit(1);
    }

    let subcmd = args[0].as_str();
    let rest = &args[1..];

    match subcmd {
        "status" => git_status(rest),
        "log" => git_log(rest),
        "diff" => git_diff(rest),
        "branch" => git_branch(rest),
        "show" => git_show(rest),
        _ => {
            // Passthrough for unhandled subcommands (commit, push, etc.)
            let status = Command::new("git").args(args).status().unwrap_or_else(|e| {
                eprintln!("git: {}", e);
                std::process::exit(1);
            });
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

/// `ig git status` — porcelain-based compact output
fn git_status(args: &[String]) {
    let native = run_git_capture(&["status", "--porcelain=v1"]);
    let native_full = run_git_capture(&[&["status"], args_as_str(args).as_slice()].concat());

    if native.trim().is_empty() {
        let output = "Clean working tree\n";
        print!("{}", output);
        track(
            "ig git status",
            native_full.len() as u64,
            output.len() as u64,
        );
        return;
    }

    // Group by status
    let mut modified = Vec::new();
    let mut added = Vec::new();
    let mut deleted = Vec::new();
    let mut untracked = Vec::new();
    let mut other = Vec::new();

    for line in native.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let file = line[3..].trim();
        match status.trim() {
            "M" | "MM" | "AM" => modified.push(file),
            "A" | "A " => added.push(file),
            "D" | " D" => deleted.push(file),
            "??" => untracked.push(file),
            _ => other.push(format!("{} {}", status.trim(), file)),
        }
    }

    let mut output = String::new();
    if !modified.is_empty() {
        output.push_str(&format!(
            "Modified ({}): {}\n",
            modified.len(),
            modified.join(", ")
        ));
    }
    if !added.is_empty() {
        output.push_str(&format!("Added ({}): {}\n", added.len(), added.join(", ")));
    }
    if !deleted.is_empty() {
        output.push_str(&format!(
            "Deleted ({}): {}\n",
            deleted.len(),
            deleted.join(", ")
        ));
    }
    if !untracked.is_empty() {
        if untracked.len() <= 5 {
            output.push_str(&format!(
                "Untracked ({}): {}\n",
                untracked.len(),
                untracked.join(", ")
            ));
        } else {
            output.push_str(&format!(
                "Untracked ({}): {}, ... +{} more\n",
                untracked.len(),
                untracked[..3].join(", "),
                untracked.len() - 3
            ));
        }
    }
    for o in &other {
        output.push_str(&format!("{}\n", o));
    }

    print!("{}", output);
    track(
        "ig git status",
        native_full.len() as u64,
        output.len() as u64,
    );
}

/// `ig git log` — compact oneline + collapsed shortstat (no per-file blocks).
fn git_log(args: &[String]) {
    let native_full = run_git_capture(&[&["log"], args_as_str(args).as_slice()].concat());

    // Detect verbose flags that explode size (per-file blocks, full diffs).
    // We collapse them all to a single `--shortstat` line per commit.
    // Flags that trigger per-file blocks → collapse to one shortstat line.
    let stat_flags = [
        "--stat",
        "--numstat",
        "--name-only",
        "--name-status",
        "--shortstat",
        "--patch",
        "-p",
        "--patch-with-stat",
        "--raw",
    ];
    // Cosmetic format flags that override our --format= → just strip them.
    let cosmetic_flags = ["--oneline", "--graph"];
    let user_wants_stat = args.iter().any(|a| {
        let head = a.split('=').next().unwrap_or(a);
        stat_flags.contains(&head)
    });
    let verbose_flags: Vec<&str> = stat_flags
        .iter()
        .chain(cosmetic_flags.iter())
        .copied()
        .collect();

    let mut cmd_args = vec!["log".to_string(), "--no-color".to_string()];

    // Compact custom format. --oneline → tightest "%h %s".
    let user_wants_oneline = args.iter().any(|a| {
        let head = a.split('=').next().unwrap_or(a);
        cosmetic_flags.contains(&head)
    });
    if user_wants_oneline {
        cmd_args.push("--format=%h %s".to_string());
    } else {
        cmd_args.push("--format=%h %s (%ar) <%an>".to_string());
    }
    if user_wants_stat {
        cmd_args.push("--shortstat".to_string());
    }

    // Default cap if user didn't specify one.
    let has_limit = args.iter().any(|a| {
        (a.starts_with('-') && a.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
            || a == "--max-count"
            || a == "-n"
            || a.starts_with("--max-count=")
    });
    if !has_limit {
        cmd_args.push("-10".to_string());
    }

    // Pass through user args except: format/pretty (we set our own), and the
    // verbose flags we already collapsed to --shortstat.
    for arg in args {
        let head = arg.split('=').next().unwrap_or(arg);
        if arg.starts_with("--format") || arg.starts_with("--pretty") {
            continue;
        }
        if verbose_flags.contains(&head) {
            continue;
        }
        cmd_args.push(arg.clone());
    }

    let raw_unbounded = run_git_capture(&cmd_args.iter().map(|s| s.as_str()).collect::<Vec<_>>());

    // Per-line width cap: long subjects (merge commits, generated messages) are
    // mostly noise past 120 chars.
    const MAX_LINE: usize = 120;
    let raw: String = raw_unbounded
        .lines()
        .map(|l| {
            if l.chars().count() > MAX_LINE {
                let mut s: String = l.chars().take(MAX_LINE - 1).collect();
                s.push('…');
                s
            } else {
                l.to_string()
            }
        })
        .map(|l| l + "\n")
        .collect();

    // Hard byte cap for large logs: keep first N commits, indicate truncation.
    const MAX_BYTES: usize = 16 * 1024;
    let output = if raw.len() <= MAX_BYTES {
        raw
    } else {
        let mut acc = String::with_capacity(MAX_BYTES + 64);
        let mut commits = 0usize;
        for line in raw.lines() {
            if acc.len() + line.len() + 1 > MAX_BYTES {
                break;
            }
            // Heuristic: a "header" line starts with a 7-12 hex sha + space.
            if !line.starts_with(' ') && line.len() > 8 {
                commits += 1;
            }
            acc.push_str(line);
            acc.push('\n');
        }
        acc.push_str(&format!("... truncated after {} commits\n", commits));
        acc
    };

    print!("{}", output);
    track("ig git log", native_full.len() as u64, output.len() as u64);
}

/// `ig git diff` — stat summary first, then compact diff
fn git_diff(args: &[String]) {
    let native_full = run_git_capture(&[&["diff"], args_as_str(args).as_slice()].concat());

    if native_full.trim().is_empty() {
        let output = "No changes\n";
        print!("{}", output);
        track("ig git diff", 0, output.len() as u64);
        return;
    }

    // Show stat first
    let stat = run_git_capture(
        &[
            &["diff", "--stat", "--no-color"],
            args_as_str(args).as_slice(),
        ]
        .concat(),
    );

    // If the full diff is small enough, show it entirely
    let output = if native_full.len() < 8000 {
        format!("{}\n{}", stat.trim_end(), native_full)
    } else {
        // Large diff: show stat + truncated diff
        let lines: Vec<&str> = native_full.lines().collect();
        let truncated: String = lines.iter().take(200).map(|l| format!("{}\n", l)).collect();
        format!(
            "{}\n{}\n... truncated ({} lines total, showing first 200)\n",
            stat.trim_end(),
            truncated.trim_end(),
            lines.len()
        )
    };

    print!("{}", output);
    track("ig git diff", native_full.len() as u64, output.len() as u64);
}

/// `ig git branch` — passthrough (already compact)
fn git_branch(args: &[String]) {
    let output = run_git_capture(&[&["branch"], args_as_str(args).as_slice()].concat());
    print!("{}", output);
    let command = if args.is_empty() {
        "ig git branch".to_string()
    } else {
        format!("ig git branch {}", args.join(" "))
    };
    tracking::log_usage(command);
}

/// `ig git show` — stat + compact diff
fn git_show(args: &[String]) {
    let native_full = run_git_capture(&[&["show"], args_as_str(args).as_slice()].concat());

    // Compact: stat + limited diff
    let stat = run_git_capture(
        &[
            &[
                "show",
                "--stat",
                "--no-color",
                "--format=%h %s (%ar) <%an>%n",
            ],
            args_as_str(args).as_slice(),
        ]
        .concat(),
    );

    let output = if native_full.len() < 8000 {
        // Small enough — show full output
        native_full.clone()
    } else {
        // Large — show stat + truncated
        let diff_start = native_full.find("\ndiff ").unwrap_or(native_full.len());
        let diff_part = &native_full[diff_start..];
        let diff_lines: Vec<&str> = diff_part.lines().collect();
        let truncated: String = diff_lines
            .iter()
            .take(150)
            .map(|l| format!("{}\n", l))
            .collect();
        format!(
            "{}\n{}\n... truncated ({} lines total)\n",
            stat.trim_end(),
            truncated.trim_end(),
            diff_lines.len()
        )
    };

    print!("{}", output);
    track("ig git show", native_full.len() as u64, output.len() as u64);
}

// ── Helpers ──

fn run_git_capture(args: &[&str]) -> String {
    let output = Command::new("git").args(args).output().unwrap_or_else(|e| {
        eprintln!("git: {}", e);
        std::process::exit(1);
    });

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            eprint!("{}", stderr);
        }
    }

    String::from_utf8_lossy(&output.stdout).to_string()
}

fn args_as_str(args: &[String]) -> Vec<&str> {
    args.iter().map(|s| s.as_str()).collect()
}

fn track(command: &str, original_bytes: u64, output_bytes: u64) {
    tracking::log_savings(&tracking::TrackEntry {
        command: command.to_string(),
        original_bytes,
        output_bytes,
        project: tracking::current_project(),
    });
}
