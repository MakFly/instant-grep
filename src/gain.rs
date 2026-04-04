//! Token savings dashboard — displays aggregated stats from tracking history.

use std::collections::BTreeMap;

use crate::tracking;
use crate::util::format_bytes;

pub fn show_gain(clear: bool, history: bool, json: bool) {
    if clear {
        tracking::clear_history();
        eprintln!("History cleared.");
        return;
    }

    let entries = tracking::read_history();

    if entries.is_empty() {
        eprintln!("No ig commands tracked yet.");
        eprintln!("Run ig with the rewrite hook installed to start tracking.");
        return;
    }

    if json {
        // Use same calculation as dashboard: input - output (not stored saved field)
        let total_input: u64 = entries.iter().map(|e| e.original_bytes).sum();
        let total_output: u64 = entries.iter().map(|e| e.output_bytes).sum();
        let total_saved = total_input.saturating_sub(total_output);
        let brain = tracking::read_brain_stats();
        println!(
            "{{\"total_tokens_saved\":{},\"total_input\":{},\"total_commands\":{},\"brain_injections\":{},\"brain_memories\":{},\"brain_tokens_saved\":{}}}",
            total_saved,
            total_input,
            entries.len(),
            brain.injections,
            brain.memories_served,
            brain.estimated_tokens_saved,
        );
        return;
    }

    if history {
        show_history(&entries);
        return;
    }

    // Aggregate totals
    let total_commands = entries.len();
    let total_input: u64 = entries.iter().map(|e| e.original_bytes).sum();
    let total_output: u64 = entries.iter().map(|e| e.output_bytes).sum();
    let total_saved = total_input.saturating_sub(total_output);
    let total_pct = if total_input > 0 {
        (total_saved as f64 / total_input as f64) * 100.0
    } else {
        0.0
    };

    // Aggregate by command prefix (ig read, ig ls, ig search, etc.)
    let mut by_cmd: BTreeMap<String, CmdStats> = BTreeMap::new();
    for entry in &entries {
        let key = command_key(&entry.command);
        let stats = by_cmd.entry(key).or_default();
        stats.count += 1;
        stats.input_bytes += entry.original_bytes;
        stats.saved_bytes += entry.saved_bytes;
    }

    // Print dashboard
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

    // Efficiency meter
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

        // Impact bar (relative to top saver)
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

    // Brain.dev savings
    let brain = tracking::read_brain_stats();
    if brain.injections > 0 {
        eprintln!();
        eprintln!("\x1b[1mbrain.dev savings\x1b[0m");
        eprintln!("────────────────────────────────────────────────────────────");
        eprintln!("  Injections:      {} prompts", format_number(brain.injections));
        eprintln!("  Memories served: {}", format_number(brain.memories_served));
        let dollars = brain.estimated_tokens_saved as f64 / 1000.0 * 0.02;
        eprintln!(
            "  Est. tokens saved: {} (~${:.2})",
            format_number(brain.estimated_tokens_saved),
            dollars,
        );
        eprintln!("────────────────────────────────────────────────────────────");
    }
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn show_history(entries: &[tracking::HistoryEntry]) {
    eprintln!("\x1b[1mig Command History\x1b[0m");
    eprintln!("════════════════════════════════════════════════════════════════════════════");
    eprintln!(
        "{:<20} {:<35} {:>8} {:>8} {:>6}",
        "Time", "Command", "Input", "Saved", "Pct"
    );
    eprintln!("────────────────────────────────────────────────────────────────────────────");

    // Show last 50 entries, most recent first
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

fn format_timestamp(ts: u64) -> String {
    if ts == 0 {
        return "unknown".to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
}
