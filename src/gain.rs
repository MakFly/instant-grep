//! Token savings dashboard — displays aggregated stats from tracking history.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::tracking;
use crate::util::format_tokens;

/// Options for the gain dashboard.
pub struct GainOpts {
    pub clear: bool,
    pub full: bool,
    pub history: bool,
    pub json: bool,
    pub project: bool,
    pub graph: bool,
    pub quota: bool,
    pub tier: String,
    pub daily: bool,
    pub weekly: bool,
    pub monthly: bool,
}

pub fn show_gain(opts: GainOpts) {
    if opts.clear {
        tracking::clear_history();
        println!("History cleared.");
        return;
    }

    let all_entries = tracking::read_history();

    if all_entries.is_empty() {
        println!("No tracking data yet.");
        println!("Run ig with the rewrite hook installed to start tracking.");
        return;
    }

    let entries: Vec<&tracking::HistoryEntry> = if opts.project {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        all_entries.iter().filter(|e| e.project == cwd).collect()
    } else {
        all_entries.iter().collect()
    };

    if entries.is_empty() {
        println!("No ig commands tracked for this project.");
        return;
    }

    if opts.json {
        let total_input: u64 = entries.iter().map(|e| e.original_bytes).sum();
        let total_output: u64 = entries.iter().map(|e| e.output_bytes).sum();
        let total_saved = total_input.saturating_sub(total_output);
        println!(
            "{{\"total_tokens_saved\":{},\"total_input\":{},\"total_commands\":{}}}",
            total_saved,
            total_input,
            entries.len(),
        );
        return;
    }

    if opts.history {
        show_history_refs(&entries);
        return;
    }

    if opts.graph {
        show_graph(&entries);
        return;
    }

    if opts.quota {
        show_quota(&entries, &opts.tier);
        return;
    }

    if opts.daily {
        show_breakdown(&entries, GroupBy::Day);
        return;
    }

    if opts.weekly {
        show_breakdown(&entries, GroupBy::Week);
        return;
    }

    if opts.monthly {
        show_breakdown(&entries, GroupBy::Month);
        return;
    }

    show_dashboard(&entries, opts.full, opts.project);
}

fn show_dashboard(entries: &[&tracking::HistoryEntry], full: bool, project_scoped: bool) {
    let total_commands = entries.len();
    let total_input: u64 = entries.iter().map(|e| e.original_bytes).sum();
    let total_output: u64 = entries.iter().map(|e| e.output_bytes).sum();
    let total_saved = total_input.saturating_sub(total_output);
    let total_pct = if total_input > 0 {
        (total_saved as f64 / total_input as f64) * 100.0
    } else {
        0.0
    };

    let mut by_cmd: BTreeMap<String, CmdStats> = BTreeMap::new();
    for entry in entries {
        let key = command_key(&entry.command);
        let stats = by_cmd.entry(key).or_default();
        stats.count += 1;
        stats.input_bytes += entry.original_bytes;
        stats.saved_bytes += entry.saved_bytes;
    }

    let title = if project_scoped {
        "ig Token Savings (Project Scope)"
    } else {
        "ig Token Savings (Global Scope)"
    };
    println!("{}", styled(title));
    println!("{}", "═".repeat(60));
    if project_scoped {
        if let Ok(cwd) = std::env::current_dir() {
            println!("Scope: {}", shorten_path(&cwd.to_string_lossy()));
        }
    }
    println!();

    print_kpi("Total commands", &total_commands.to_string());
    print_kpi("Input tokens", &format_tokens(total_input));
    print_kpi("Output tokens", &format_tokens(total_output));
    print_kpi(
        "Tokens saved",
        &format!(
            "{} ({:.1}%)",
            format_tokens(total_saved),
            total_pct
        ),
    );
    print_efficiency_meter(total_pct);
    println!();

    let mut sorted: Vec<_> = by_cmd.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1.saved_bytes));
    println!("{}", styled("By Command"));

    let cmd_width = 24usize;
    let impact_width = 10usize;
    let count_width = sorted
        .iter()
        .map(|(_, stats)| stats.count.to_string().len())
        .max()
        .unwrap_or(5)
        .max(5);
    let saved_width = sorted
        .iter()
        .map(|(_, stats)| format_tokens(stats.saved_bytes).len())
        .max()
        .unwrap_or(5)
        .max(5);
    let table_width =
        3 + 2 + cmd_width + 2 + count_width + 2 + saved_width + 2 + 6 + 2 + impact_width;

    println!("{}", "─".repeat(table_width));
    println!(
        "{:>3}  {:<cmd_width$}  {:>count_width$}  {:>saved_width$}  {:>6}  {:<impact_width$}",
        "#",
        "Command",
        "Count",
        "Saved",
        "Avg%",
        "Impact",
        cmd_width = cmd_width,
        count_width = count_width,
        saved_width = saved_width,
        impact_width = impact_width,
    );
    println!("{}", "─".repeat(table_width));

    let max_saved = sorted
        .first()
        .map(|(_, stats)| stats.saved_bytes)
        .unwrap_or(1)
        .max(1);
    let visible_count = if full {
        sorted.len()
    } else {
        sorted.len().min(20)
    };

    for (i, (cmd, stats)) in sorted.iter().take(visible_count).enumerate() {
        let avg_pct = if stats.input_bytes > 0 {
            (stats.saved_bytes as f64 / stats.input_bytes as f64) * 100.0
        } else {
            0.0
        };
        let row_idx = format!("{:>2}.", i + 1);
        let cmd_cell = style_command_cell(&truncate_for_column(cmd, cmd_width));
        let pct_plain = format!("{:>6}", format!("{:.1}%", avg_pct));

        println!(
            "{}  {}  {:>count_width$}  {:>saved_width$}  {}  {}",
            row_idx,
            cmd_cell,
            stats.count,
            format_tokens(stats.saved_bytes),
            colorize_pct(avg_pct, &pct_plain),
            mini_bar(stats.saved_bytes, max_saved, impact_width),
            count_width = count_width,
            saved_width = saved_width,
        );
    }
    println!("{}", "─".repeat(table_width));
    if !full && sorted.len() > visible_count {
        println!(
            "Showing top {} of {} commands. Use `ig gain --full` to see the full list.",
            visible_count,
            sorted.len()
        );
    }

    let usage_only: Vec<_> = sorted
        .iter()
        .filter(|(_, stats)| stats.saved_bytes == 0 && stats.count > 0)
        .collect();
    if !usage_only.is_empty() && !full {
        let mut by_count: Vec<_> = usage_only.clone();
        by_count.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.count));
        let visible = by_count.len().min(10);
        println!();
        println!("{}", styled("By Usage (no token baseline)"));
        let usage_width = 3 + 2 + cmd_width + 2 + count_width;
        println!("{}", "─".repeat(usage_width));
        println!(
            "{:>3}  {:<cmd_width$}  {:>count_width$}",
            "#",
            "Command",
            "Count",
            cmd_width = cmd_width,
            count_width = count_width,
        );
        println!("{}", "─".repeat(usage_width));
        for (i, (cmd, stats)) in by_count.iter().take(visible).enumerate() {
            let row_idx = format!("{:>2}.", i + 1);
            let cmd_cell = style_command_cell(&truncate_for_column(cmd, cmd_width));
            println!(
                "{}  {}  {:>count_width$}",
                row_idx,
                cmd_cell,
                stats.count,
                count_width = count_width,
            );
        }
        println!("{}", "─".repeat(usage_width));
        if by_count.len() > visible {
            println!(
                "Showing top {} of {} usage-only commands. Use `ig gain --full` for all.",
                visible,
                by_count.len()
            );
        }
    }
}

fn show_history_refs(entries: &[&tracking::HistoryEntry]) {
    println!("{}", styled("ig Command History"));
    println!("{}", "═".repeat(60));
    println!(
        "{:<20} {:<35} {:>8} {:>8} {:>6}",
        "Time", "Command", "Input", "Saved", "Pct"
    );
    println!("{}", "─".repeat(60));

    for entry in entries.iter().rev().take(50) {
        let time_str = format_timestamp(entry.timestamp);
        let cmd = if entry.command.len() > 33 {
            let cut = entry.command.floor_char_boundary(30);
            format!("{}...", &entry.command[..cut])
        } else {
            entry.command.clone()
        };
        let pct = if entry.original_bytes > 0 {
            (entry.saved_bytes as f64 / entry.original_bytes as f64) * 100.0
        } else {
            0.0
        };
        let sign = if pct >= 70.0 {
            "▲"
        } else if pct >= 30.0 {
            "■"
        } else {
            "•"
        };
        println!(
            "{:<20} {} {:<33} {:>8} {:>8} {:>5.1}%",
            time_str,
            sign,
            cmd,
            format_tokens(entry.original_bytes),
            format_tokens(entry.saved_bytes),
            pct,
        );
    }
    println!("{}", "─".repeat(60));
    println!(
        "Showing last {} of {} entries",
        entries.len().min(50),
        entries.len()
    );
}

/// ASCII bar chart of daily savings for the last 14 days.
fn show_graph(entries: &[&tracking::HistoryEntry]) {
    let now = now_secs();
    let fourteen_days_ago = now.saturating_sub(14 * 86400);

    let mut daily: BTreeMap<String, u64> = BTreeMap::new();
    for entry in entries {
        if entry.timestamp < fourteen_days_ago {
            continue;
        }
        let date = date_from_ts(entry.timestamp);
        *daily.entry(date).or_default() += entry.saved_bytes;
    }

    if daily.is_empty() {
        println!("No data in the last 14 days.");
        return;
    }

    let mut days: Vec<(String, u64)> = Vec::new();
    for i in 0..14 {
        let ts = now.saturating_sub(i * 86400);
        let date = date_from_ts(ts);
        let label = short_date_from_ts(ts);
        let saved = daily.get(&date).copied().unwrap_or(0);
        days.push((label, saved));
    }
    days.reverse();

    let max_saved = days.iter().map(|(_, s)| *s).max().unwrap_or(1).max(1);
    let bar_max: usize = 40;

    println!("{}", styled("Daily Savings (last 14 days)"));
    println!("{}", "─".repeat(58));
    for (label, saved) in &days {
        let bar_len = ((*saved as f64 / max_saved as f64) * bar_max as f64) as usize;
        let bar = "█".repeat(bar_len);
        let spaces = " ".repeat(bar_max - bar_len);
        if *saved > 0 {
            println!("{} │{}{} {}", label, bar, spaces, format_tokens(*saved));
        } else {
            println!("{} │{} -", label, " ".repeat(bar_max));
        }
    }
    println!("{}", "─".repeat(58));
}

/// Monthly quota estimate based on savings.
fn show_quota(entries: &[&tracking::HistoryEntry], tier: &str) {
    let now = now_secs();

    let thirty_days_ago = now.saturating_sub(30 * 86400);
    let recent: Vec<&&tracking::HistoryEntry> = entries
        .iter()
        .filter(|e| e.timestamp >= thirty_days_ago)
        .collect();

    if recent.is_empty() {
        println!("No data in the last 30 days.");
        return;
    }

    let total_saved: u64 = recent.iter().map(|e| e.saved_bytes).sum();

    let min_ts = recent.iter().map(|e| e.timestamp).min().unwrap_or(now);
    let span_days = ((now - min_ts) as f64 / 86400.0).max(1.0);
    let daily_avg = total_saved as f64 / span_days;
    let monthly_saved = daily_avg * 30.0;

    let monthly_tokens = monthly_saved / 4.0;

    let opus_turn_tokens: f64 = 400_000.0;
    let equiv_messages = monthly_tokens / opus_turn_tokens;

    let (tier_label, tier_desc) = match tier {
        "pro" => ("Pro", "Pro ($20/mo)"),
        "5x" => ("Max 5x", "Max 5x ($100/mo)"),
        _ => ("Max 20x", "Max 20x ($200/mo)"),
    };

    println!("{}", styled(&format!("Monthly Quota Estimate ({} tier)", tier_label)));
    println!("{}", "─".repeat(58));
    print_kpi(
        "Monthly projection",
        &format!("{:.1}M tokens", monthly_tokens / 1_000_000.0),
    );
    print_kpi(
        "Messages saved",
        &format!("~{:.0} Opus turns", equiv_messages),
    );
    print_kpi("Subscription tier", tier_desc);
    println!("{}", "─".repeat(58));
}

#[derive(Clone, Copy)]
enum GroupBy {
    Day,
    Week,
    Month,
}

/// Show a breakdown table grouped by day, week, or month.
fn show_breakdown(entries: &[&tracking::HistoryEntry], group: GroupBy) {
    let title = match group {
        GroupBy::Day => "Daily Breakdown",
        GroupBy::Week => "Weekly Breakdown",
        GroupBy::Month => "Monthly Breakdown",
    };

    let mut groups: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    for entry in entries {
        let key = match group {
            GroupBy::Day => date_from_ts(entry.timestamp),
            GroupBy::Week => week_from_ts(entry.timestamp),
            GroupBy::Month => month_from_ts(entry.timestamp),
        };
        let stats = groups.entry(key).or_default();
        stats.0 += 1;
        stats.1 += entry.original_bytes;
        stats.2 += entry.saved_bytes;
    }

    let date_header = match group {
        GroupBy::Day => "Date",
        GroupBy::Week => "Week",
        GroupBy::Month => "Month",
    };

    println!("{}", styled(title));
    println!("{}", "─".repeat(46));
    println!(
        "  {:<14} {:>8}  {:>8}  {:>5}",
        date_header, "Commands", "Saved", "Avg%"
    );
    println!("{}", "─".repeat(46));

    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.0.clone()));

    for (key, (count, input, saved)) in sorted.iter().take(30) {
        let avg_pct = if *input > 0 {
            (*saved as f64 / *input as f64) * 100.0
        } else {
            0.0
        };
        let pct_plain = format!("{:.1}%", avg_pct);
        println!(
            "  {:<14} {:>8}  {:>8}  {}",
            key,
            count,
            format_tokens(*saved),
            colorize_pct(avg_pct, &format!("{:>5}", pct_plain)),
        );
    }
    println!("{}", "─".repeat(46));
}

// ── Date helpers ─────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn date_from_ts(ts: u64) -> String {
    let (y, m, d) = civil_from_days(ts / 86400);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn short_date_from_ts(ts: u64) -> String {
    let (_, m, d) = civil_from_days(ts / 86400);
    let month_name = match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    };
    format!("{} {:2}", month_name, d)
}

fn month_from_ts(ts: u64) -> String {
    let (y, m, _) = civil_from_days(ts / 86400);
    format!("{:04}-{:02}", y, m)
}

fn week_from_ts(ts: u64) -> String {
    let days = ts / 86400;
    let dow = ((days + 3) % 7) as i64;
    let thursday = days as i64 - dow + 3;
    let (y, m, _) = civil_from_days(thursday as u64);
    let jan1_days = days_from_civil(y, 1, 1);
    let week_num = ((thursday - jan1_days as i64) / 7) + 1;
    let iso_year = if m == 1 && week_num > 50 { y - 1 } else { y };
    format!("{:04}-W{:02}", iso_year, week_num)
}

/// Algorithm from Howard Hinnant (public domain).
fn civil_from_days(days: u64) -> (i32, u32, u32) {
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> u64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let m = m as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe as i64 - 719468;
    days as u64
}

fn format_timestamp(ts: u64) -> String {
    if ts == 0 {
        return "unknown".to_string();
    }
    let now = now_secs();
    let ago = now.saturating_sub(ts);
    if ago < 60 {
        format!("{}s ago", ago)
    } else if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else if ago < 86400 {
        format!("{}h ago", ago / 3600)
    } else {
        format!("{}d ago", ago / 86400)
    }
}

#[derive(Default)]
struct CmdStats {
    count: u64,
    input_bytes: u64,
    saved_bytes: u64,
}

/// Extract a normalized command key.
fn command_key(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return "unknown".to_string();
    }

    if parts.first() != Some(&"ig") {
        return raw_command_key(&parts);
    }

    match parts.get(1) {
        Some(&"run") => {
            let key = raw_command_key(&parts[2..]);
            if key == "unknown" {
                "ig run".to_string()
            } else {
                format!("ig run {}", key)
            }
        }
        Some(&"git") => match first_non_flag(&parts[2..]) {
            Some(sub) => format!("ig git {}", sub),
            None => "ig git".to_string(),
        },
        Some(&"read") => read_command_key(&parts[2..]),
        Some(&"json") => {
            if parts[2..].contains(&"--schema") {
                "ig json --schema".to_string()
            } else {
                "ig json".to_string()
            }
        }
        Some(&"err") => {
            let key = raw_command_key(&parts[2..]);
            if key == "unknown" {
                "ig err".to_string()
            } else {
                format!("ig err {}", key)
            }
        }
        Some(sub) if !sub.starts_with('"') && !sub.starts_with('\'') && !sub.starts_with('-') => {
            format!("ig {}", sub)
        }
        _ => "ig search".to_string(),
    }
}

fn read_command_key(args: &[&str]) -> String {
    let mut flags = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i];
        if !arg.starts_with('-') {
            break;
        }
        match arg {
            "-s" | "-a" | "-d" | "--plain" => flags.push(arg),
            "-b" => {
                flags.push("-b");
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if flags.is_empty() {
        "ig read".to_string()
    } else {
        format!("ig read {}", flags.join(" "))
    }
}

fn raw_command_key(parts: &[&str]) -> String {
    if parts.is_empty() {
        return "unknown".to_string();
    }

    let base = parts[0];
    match base {
        "git" => format_with_optional_subcommand("git", first_non_flag(&parts[1..])),
        "cargo" | "bun" | "bunx" | "go" | "dotnet" | "uv" | "pytest" | "ruff" | "mypy"
        | "eslint" | "tsc" | "vitest" | "make" | "just" => {
            format_with_optional_subcommand(base, first_non_flag(&parts[1..]))
        }
        "npm" | "pnpm" | "npx" => format_package_manager_key(base, &parts[1..]),
        "docker" => format_docker_key(&parts[1..]),
        "kubectl" => format_with_optional_subcommand("kubectl", first_non_flag(&parts[1..])),
        "gh" => format_gh_key(&parts[1..]),
        _ => base.to_string(),
    }
}

fn format_with_optional_subcommand(base: &str, sub: Option<&str>) -> String {
    match sub {
        Some(sub) => format!("{} {}", base, sub),
        None => base.to_string(),
    }
}

fn format_package_manager_key(base: &str, args: &[&str]) -> String {
    match first_non_flag(args) {
        Some("run") | Some("exec") => {
            let mut key = format!("{} {}", base, first_non_flag(args).unwrap_or("run"));
            if let Some(script) = first_non_flag_after(args, 1) {
                key.push(' ');
                key.push_str(script);
            }
            key
        }
        Some(sub) => format!("{} {}", base, sub),
        None => base.to_string(),
    }
}

fn format_docker_key(args: &[&str]) -> String {
    match first_non_flag(args) {
        Some("compose") => match first_non_flag_after(args, 1) {
            Some(sub) => format!("docker compose {}", sub),
            None => "docker compose".to_string(),
        },
        Some(sub) => format!("docker {}", sub),
        None => "docker".to_string(),
    }
}

fn format_gh_key(args: &[&str]) -> String {
    match first_non_flag(args) {
        Some(top) => match first_non_flag_after(args, 1) {
            Some(sub) => format!("gh {} {}", top, sub),
            None => format!("gh {}", top),
        },
        None => "gh".to_string(),
    }
}

fn first_non_flag<'a>(parts: &'a [&'a str]) -> Option<&'a str> {
    parts.iter().copied().find(|part| !part.starts_with('-'))
}

fn first_non_flag_after<'a>(parts: &'a [&'a str], after_non_flag_index: usize) -> Option<&'a str> {
    let mut seen = 0usize;
    for part in parts {
        if part.starts_with('-') {
            continue;
        }
        if seen == after_non_flag_index {
            return Some(*part);
        }
        seen += 1;
    }
    None
}

// ── Display helpers (TTY-aware) ──────────────────────────────────────────

fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var("NO_COLOR").is_err()
}

fn ansi(text: &str, code: &str) -> String {
    if use_color() {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

fn styled(text: &str) -> String {
    ansi(text, "1;32")
}

fn style_command_cell(text: &str) -> String {
    ansi(text, "1;96")
}

fn colorize_pct(pct: f64, text: &str) -> String {
    if pct >= 70.0 {
        ansi(text, "1;32")
    } else if pct >= 40.0 {
        ansi(text, "1;33")
    } else {
        ansi(text, "1;31")
    }
}

fn print_kpi(label: &str, value: &str) {
    println!("{:<18} {}", format!("{}:", label), value);
}

fn print_efficiency_meter(pct: f64) {
    let width = 24usize;
    let filled = (((pct / 100.0) * width as f64).round() as usize).min(width);
    let meter = format!("{}{}", "█".repeat(filled), "░".repeat(width - filled));
    let pct_str = format!("{pct:.1}%");
    println!(
        "{:<18} {} {}",
        "Efficiency meter:",
        ansi(&meter, "32"),
        colorize_pct(pct, &pct_str)
    );
}

fn truncate_for_column(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= width {
        return format!("{:<width$}", text, width = width);
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }
    let mut out: String = text.chars().take(width - 3).collect();
    out.push_str("...");
    out
}

fn mini_bar(value: u64, max: u64, width: usize) -> String {
    if width == 0 || max == 0 {
        return String::new();
    }

    let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(width - filled));
    ansi(&bar, "36")
}

fn shorten_path(path: &str) -> String {
    let path_buf = PathBuf::from(path);
    let comps: Vec<String> = path_buf
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if comps.len() <= 4 {
        return path.to_string();
    }
    let root = comps[0].as_str();
    if root == "/" || root.is_empty() {
        format!("/.../{}/{}", comps[comps.len() - 2], comps[comps.len() - 1])
    } else {
        format!(
            "{}/.../{}/{}",
            root,
            comps[comps.len() - 2],
            comps[comps.len() - 1]
        )
    }
}

/// Return the [start, end) unix-second range for a named period.
fn period_range(name: &str, now: u64) -> Option<(u64, u64)> {
    const DAY: u64 = 86400;
    let today = (now / DAY) * DAY;
    let week_start = today.saturating_sub(6 * DAY);
    let month_start = today.saturating_sub(29 * DAY);
    match name {
        "this-day" => Some((today, now)),
        "last-day" => Some((today.saturating_sub(DAY), today)),
        "this-week" => Some((week_start, now)),
        "last-week" => Some((week_start.saturating_sub(7 * DAY), week_start)),
        "this-month" => Some((month_start, now)),
        "last-month" => Some((month_start.saturating_sub(30 * DAY), month_start)),
        _ => None,
    }
}

fn sum_entries(entries: &[&tracking::HistoryEntry], range: (u64, u64)) -> (u64, u64, usize) {
    let (a, b) = range;
    let mut input = 0u64;
    let mut output = 0u64;
    let mut count = 0usize;
    for e in entries {
        if e.timestamp >= a && e.timestamp < b {
            input += e.original_bytes;
            output += e.output_bytes;
            count += 1;
        }
    }
    (input, output, count)
}

/// Compare token savings between two named periods.
pub fn show_compare(spec: &str, json: bool) {
    let parts: Vec<&str> = spec.splitn(2, ':').collect();
    if parts.len() != 2 {
        println!(
            "Invalid --compare value: {}. Expected format: this-week:last-week",
            spec
        );
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let Some(left) = period_range(parts[0], now) else {
        println!("Unknown period: {}", parts[0]);
        return;
    };
    let Some(right) = period_range(parts[1], now) else {
        println!("Unknown period: {}", parts[1]);
        return;
    };

    let all = tracking::read_history();
    let refs: Vec<&tracking::HistoryEntry> = all.iter().collect();

    let (li, lo, lc) = sum_entries(&refs, left);
    let (ri, ro, rc) = sum_entries(&refs, right);
    let ls = li.saturating_sub(lo);
    let rs = ri.saturating_sub(ro);

    if json {
        println!(
            "{{\"left\":{{\"name\":\"{}\",\"commands\":{},\"saved\":{}}},\"right\":{{\"name\":\"{}\",\"commands\":{},\"saved\":{}}}}}",
            parts[0], lc, ls, parts[1], rc, rs
        );
        return;
    }

    println!(
        "{}  {} vs {}",
        styled("gain --compare"),
        parts[0],
        parts[1]
    );
    println!("{}", "─".repeat(60));
    println!(
        "  {:<18} {:>6}  {:>10}  {:>10}",
        "Period", "Count", "Saved", "Input"
    );
    println!("{}", "─".repeat(60));
    println!(
        "  {:<18} {:>6}  {:>10}  {:>10}",
        parts[0],
        lc,
        format_tokens(ls),
        format_tokens(li)
    );
    println!(
        "  {:<18} {:>6}  {:>10}  {:>10}",
        parts[1],
        rc,
        format_tokens(rs),
        format_tokens(ri)
    );
    println!("{}", "─".repeat(60));
    let delta_count = lc as i64 - rc as i64;
    let delta_saved = ls as i64 - rs as i64;
    let sign = if delta_saved >= 0 { "+" } else { "" };
    println!(
        "  Δ commands: {:+}    Δ saved: {}{}",
        delta_count,
        sign,
        format_tokens(delta_saved.unsigned_abs())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_key() {
        assert_eq!(command_key("ig read src/main.rs"), "ig read");
        assert_eq!(command_key("ig read -s src/main.rs"), "ig read -s");
        assert_eq!(command_key("ig ls src/"), "ig ls");
        assert_eq!(command_key("ig \"pattern\""), "ig search");
        assert_eq!(command_key("ig smart src/"), "ig smart");
        assert_eq!(command_key("ig git status"), "ig git status");
        assert_eq!(
            command_key("ig run cargo test --release"),
            "ig run cargo test"
        );
        assert_eq!(command_key("ig run npm run build"), "ig run npm run build");
        assert_eq!(
            command_key("ig run docker compose logs web"),
            "ig run docker compose logs"
        );
        assert_eq!(command_key("ig err bun test"), "ig err bun test");
    }

    #[test]
    fn test_civil_from_days() {
        let (y, m, d) = civil_from_days(20554);
        assert_eq!((y, m, d), (2026, 4, 11));
    }

    #[test]
    fn test_date_from_ts() {
        let ts = 20554 * 86400;
        assert_eq!(date_from_ts(ts), "2026-04-11");
    }

    #[test]
    fn test_month_from_ts() {
        let ts = 20554 * 86400;
        assert_eq!(month_from_ts(ts), "2026-04");
    }

    #[test]
    fn test_week_from_ts() {
        let w = week_from_ts(20554 * 86400);
        assert!(w.starts_with("2026-W"));
    }

    #[test]
    fn test_short_date() {
        let ts = 20554 * 86400;
        assert_eq!(short_date_from_ts(ts), "Apr 11");
    }

    #[test]
    fn period_range_this_day() {
        let now = 1_700_000_000;
        let (a, b) = period_range("this-day", now).unwrap();
        assert!(a <= now);
        assert_eq!(b, now);
        assert!((now - a) < 86400);
    }

    #[test]
    fn period_range_last_week() {
        let now = 1_700_000_000;
        let (a, b) = period_range("last-week", now).unwrap();
        assert!(b > a);
        assert_eq!(b - a, 7 * 86400);
    }

    #[test]
    fn period_range_unknown() {
        assert!(period_range("yesterday", 0).is_none());
    }

    #[test]
    fn sum_entries_filters_by_range() {
        let entries = [
            tracking::HistoryEntry {
                command: "ig read".into(),
                original_bytes: 1000,
                output_bytes: 200,
                saved_bytes: 800,
                timestamp: 100,
                project: "".into(),
            },
            tracking::HistoryEntry {
                command: "ig ls".into(),
                original_bytes: 500,
                output_bytes: 100,
                saved_bytes: 400,
                timestamp: 200,
                project: "".into(),
            },
            tracking::HistoryEntry {
                command: "ig git".into(),
                original_bytes: 300,
                output_bytes: 50,
                saved_bytes: 250,
                timestamp: 300,
                project: "".into(),
            },
        ];
        let refs: Vec<&tracking::HistoryEntry> = entries.iter().collect();
        let (input, output, count) = sum_entries(&refs, (150, 250));
        assert_eq!(count, 1);
        assert_eq!(input, 500);
        assert_eq!(output, 100);
    }
}
