//! Token savings dashboard — displays aggregated stats from tracking history.

use std::collections::BTreeMap;

use crate::tracking;
use crate::util::format_bytes;

/// Options for the gain dashboard.
pub struct GainOpts {
    pub clear: bool,
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
        eprintln!("History cleared.");
        return;
    }

    let all_entries = tracking::read_history();

    if all_entries.is_empty() {
        eprintln!("No ig commands tracked yet.");
        eprintln!("Run ig with the rewrite hook installed to start tracking.");
        return;
    }

    // Filter by project if requested
    let entries: Vec<&tracking::HistoryEntry> = if opts.project {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        all_entries.iter().filter(|e| e.project == cwd).collect()
    } else {
        all_entries.iter().collect()
    };

    if entries.is_empty() {
        eprintln!("No ig commands tracked for this project.");
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

    // Default dashboard
    show_dashboard(&entries);
}

fn show_dashboard(entries: &[&tracking::HistoryEntry]) {
    let total_commands = entries.len();
    let total_input: u64 = entries.iter().map(|e| e.original_bytes).sum();
    let total_output: u64 = entries.iter().map(|e| e.output_bytes).sum();
    let total_saved = total_input.saturating_sub(total_output);
    let total_pct = if total_input > 0 {
        (total_saved as f64 / total_input as f64) * 100.0
    } else {
        0.0
    };

    // Aggregate by command prefix
    let mut by_cmd: BTreeMap<String, CmdStats> = BTreeMap::new();
    for entry in entries {
        let key = command_key(&entry.command);
        let stats = by_cmd.entry(key).or_default();
        stats.count += 1;
        stats.input_bytes += entry.original_bytes;
        stats.saved_bytes += entry.saved_bytes;
    }

    eprintln!("\x1b[1mig Token Savings\x1b[0m");
    eprintln!("════════════════════════════════════════════════════════════");
    eprintln!();
    eprintln!("Total commands:    {}", total_commands);
    eprintln!("Input bytes:       {}", format_bytes(total_input));
    eprintln!("Output bytes:      {}", format_bytes(total_output));
    eprintln!(
        "Bytes saved:       {} (\x1b[32m{:.1}%\x1b[0m)",
        format_bytes(total_saved),
        total_pct
    );

    let bar_width: usize = 24;
    let filled = ((total_pct / 100.0) * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);
    eprintln!(
        "Efficiency meter:  {}{}  {:.1}%",
        "█".repeat(filled),
        "░".repeat(empty),
        total_pct
    );

    eprintln!();
    eprintln!("By Command");
    eprintln!("────────────────────────────────────────────────────────────");
    eprintln!(
        "  {:2}  {:<24} {:>5}  {:>7}  {:>5}",
        "#", "Command", "Count", "Saved", "Avg%"
    );
    eprintln!("────────────────────────────────────────────────────────────");

    let mut sorted: Vec<_> = by_cmd.into_iter().collect();
    sorted.sort_by(|a, b| b.1.saved_bytes.cmp(&a.1.saved_bytes));

    for (i, (cmd, stats)) in sorted.iter().enumerate() {
        let avg_pct = if stats.input_bytes > 0 {
            (stats.saved_bytes as f64 / stats.input_bytes as f64) * 100.0
        } else {
            0.0
        };

        let max_saved = sorted
            .first()
            .map(|(_, s)| s.saved_bytes)
            .unwrap_or(1)
            .max(1);
        let bar_len = ((stats.saved_bytes as f64 / max_saved as f64) * 10.0) as usize;
        let bar = format!(
            "{}{}",
            "█".repeat(bar_len),
            "░".repeat(10usize.saturating_sub(bar_len))
        );

        eprintln!(
            "  {:2}. {:<24} {:>5}  {:>7}  {:>4.1}%  {}",
            i + 1,
            cmd,
            stats.count,
            format_bytes(stats.saved_bytes),
            avg_pct,
            bar,
        );
    }
    eprintln!("────────────────────────────────────────────────────────────");
}

fn show_history_refs(entries: &[&tracking::HistoryEntry]) {
    eprintln!("\x1b[1mig Command History\x1b[0m");
    eprintln!("════════════════════════════════════════════════════════════════════════════");
    eprintln!(
        "{:<20} {:<35} {:>8} {:>8} {:>6}",
        "Time", "Command", "Input", "Saved", "Pct"
    );
    eprintln!("────────────────────────────────────────────────────────────────────────────");

    for entry in entries.iter().rev().take(50) {
        let time_str = format_timestamp(entry.timestamp);
        let cmd = if entry.command.len() > 33 {
            format!("{}...", &entry.command[..30])
        } else {
            entry.command.clone()
        };
        let pct = if entry.original_bytes > 0 {
            (entry.saved_bytes as f64 / entry.original_bytes as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "{:<20} {:<35} {:>8} {:>8} {:>5.1}%",
            time_str,
            cmd,
            format_bytes(entry.original_bytes),
            format_bytes(entry.saved_bytes),
            pct,
        );
    }
    eprintln!("────────────────────────────────────────────────────────────────────────────");
    eprintln!(
        "Showing last {} of {} entries",
        entries.len().min(50),
        entries.len()
    );
}

/// ASCII bar chart of daily savings for the last 14 days.
fn show_graph(entries: &[&tracking::HistoryEntry]) {
    let now = now_secs();
    let fourteen_days_ago = now.saturating_sub(14 * 86400);

    // Group saved bytes by date string
    let mut daily: BTreeMap<String, u64> = BTreeMap::new();
    for entry in entries {
        if entry.timestamp < fourteen_days_ago {
            continue;
        }
        let date = date_from_ts(entry.timestamp);
        *daily.entry(date).or_default() += entry.saved_bytes;
    }

    if daily.is_empty() {
        eprintln!("No data in the last 14 days.");
        return;
    }

    // Build ordered list of last 14 days
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
    let bar_max: usize = 30;

    eprintln!("\x1b[1mDaily Savings (last 14 days)\x1b[0m");
    eprintln!("──────────────────────────────────────────────────");
    for (label, saved) in &days {
        let bar_len = ((*saved as f64 / max_saved as f64) * bar_max as f64) as usize;
        let bar = "█".repeat(bar_len);
        if *saved > 0 {
            eprintln!("{}  {:<30}  {}", label, bar, format_bytes(*saved));
        } else {
            eprintln!("{}  {:<30}  -", label, "");
        }
    }
    eprintln!("──────────────────────────────────────────────────");
}

/// Monthly quota estimate based on savings.
fn show_quota(entries: &[&tracking::HistoryEntry], tier: &str) {
    let now = now_secs();

    // Calculate daily average from last 30 days
    let thirty_days_ago = now.saturating_sub(30 * 86400);
    let recent: Vec<&&tracking::HistoryEntry> = entries
        .iter()
        .filter(|e| e.timestamp >= thirty_days_ago)
        .collect();

    if recent.is_empty() {
        eprintln!("No data in the last 30 days.");
        return;
    }

    let total_saved: u64 = recent.iter().map(|e| e.saved_bytes).sum();

    // Find actual span in days
    let min_ts = recent.iter().map(|e| e.timestamp).min().unwrap_or(now);
    let span_days = ((now - min_ts) as f64 / 86400.0).max(1.0);
    let daily_avg = total_saved as f64 / span_days;
    let monthly_saved = daily_avg * 30.0;

    // Token conversion: 1 token ~ 4 chars (bytes)
    let monthly_tokens = monthly_saved / 4.0;

    // Opus turn ~ 400K tokens
    let opus_turn_tokens: f64 = 400_000.0;
    let equiv_messages = monthly_tokens / opus_turn_tokens;

    let (tier_label, tier_desc) = match tier {
        "pro" => ("Pro", "Pro ($20/mo)"),
        "5x" => ("Max 5x", "Max 5x ($100/mo)"),
        "20x" | _ => ("Max 20x", "Max 20x ($200/mo)"),
    };

    eprintln!(
        "\x1b[1mMonthly Quota Estimate ({} tier)\x1b[0m",
        tier_label
    );
    eprintln!("──────────────────────────────────────");
    eprintln!(
        "Monthly savings projection:  {:.1}M tokens",
        monthly_tokens / 1_000_000.0
    );
    eprintln!(
        "Equivalent messages saved:   ~{:.0} Opus turns",
        equiv_messages
    );
    eprintln!("Subscription tier:           {}", tier_desc);
    eprintln!("──────────────────────────────────────");
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

    // Group entries
    let mut groups: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new(); // (count, input, saved)
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

    eprintln!("\x1b[1m{}\x1b[0m", title);
    eprintln!("────────────────────────────────────────────");
    eprintln!(
        "  {:<14} {:>8}  {:>8}  {:>5}",
        date_header, "Commands", "Saved", "Avg%"
    );
    eprintln!("────────────────────────────────────────────");

    // Show in reverse chronological order, last 30 entries max
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|a, b| b.0.cmp(&a.0));

    for (key, (count, input, saved)) in sorted.iter().take(30) {
        let avg_pct = if *input > 0 {
            (*saved as f64 / *input as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "  {:<14} {:>8}  {:>8}  {:>4.1}%",
            key,
            count,
            format_bytes(*saved),
            avg_pct,
        );
    }
    eprintln!("────────────────────────────────────────────");
}

// ── Date helpers ─────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Convert Unix timestamp to "YYYY-MM-DD" using manual calculation.
fn date_from_ts(ts: u64) -> String {
    let (y, m, d) = civil_from_days(ts / 86400);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert Unix timestamp to short display like "Apr 11".
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

/// Convert Unix timestamp to "YYYY-MM" string.
fn month_from_ts(ts: u64) -> String {
    let (y, m, _) = civil_from_days(ts / 86400);
    format!("{:04}-{:02}", y, m)
}

/// Convert Unix timestamp to ISO week string like "2026-W15".
fn week_from_ts(ts: u64) -> String {
    let days = ts / 86400;
    // ISO week: Monday=1. Jan 1 1970 was Thursday (day 4).
    // day_of_week: 0=Monday ... 6=Sunday
    let dow = ((days + 3) % 7) as i64; // Thursday=0 at epoch, shift so Monday=0
    // ISO week date: find the Thursday of this week
    let thursday = days as i64 - dow + 3;
    let (y, m, _) = civil_from_days(thursday as u64);
    // Day of year for that Thursday
    let jan1_days = days_from_civil(y, 1, 1);
    let week_num = ((thursday - jan1_days as i64) / 7) + 1;
    // ISO year might differ at year boundaries
    let iso_year = if m == 1 && week_num > 50 {
        y - 1
    } else {
        y
    };
    format!("{:04}-W{:02}", iso_year, week_num)
}

/// Convert days since epoch to (year, month, day).
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

/// Convert (year, month, day) to days since epoch.
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

/// Extract a normalized command key (e.g., "ig read", "ig ls", "ig search")
fn command_key(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.first() != Some(&"ig") {
        return cmd
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string();
    }

    match parts.get(1) {
        Some(sub) if !sub.starts_with('"') && !sub.starts_with('\'') && !sub.starts_with('-') => {
            format!("ig {}", sub)
        }
        _ => "ig search".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_key() {
        assert_eq!(command_key("ig read src/main.rs"), "ig read");
        assert_eq!(command_key("ig ls src/"), "ig ls");
        assert_eq!(command_key("ig \"pattern\""), "ig search");
        assert_eq!(command_key("ig smart src/"), "ig smart");
    }

    #[test]
    fn test_civil_from_days() {
        // 2026-04-11 is day 20554 since epoch
        let (y, m, d) = civil_from_days(20554);
        assert_eq!((y, m, d), (2026, 4, 11));
    }

    #[test]
    fn test_date_from_ts() {
        // 2026-04-11 00:00:00 UTC = 20554 * 86400
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
}
