/// Token savings dashboard — displays aggregated stats from tracking history.

use std::collections::BTreeMap;

use crate::tracking;

pub fn show_gain(clear: bool) {
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
        stats.output_bytes += entry.output_bytes;
        stats.saved_bytes += entry.saved_bytes;
    }

    // Print dashboard
    eprintln!("\x1b[1mig Token Savings\x1b[0m");
    eprintln!("════════════════════════════════════════════════════════════");
    eprintln!();
    eprintln!(
        "Total commands:    {}",
        total_commands
    );
    eprintln!(
        "Input bytes:       {}",
        format_bytes(total_input)
    );
    eprintln!(
        "Output bytes:      {}",
        format_bytes(total_output)
    );
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
        let max_saved = sorted.first().map(|(_, s)| s.saved_bytes).unwrap_or(1);
        let bar_len = if max_saved > 0 {
            ((stats.saved_bytes as f64 / max_saved as f64) * 10.0) as usize
        } else {
            0
        };
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

#[derive(Default)]
struct CmdStats {
    count: u64,
    input_bytes: u64,
    output_bytes: u64,
    saved_bytes: u64,
}

/// Extract a normalized command key (e.g., "ig read", "ig ls", "ig search")
fn command_key(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.first() != Some(&"ig") {
        return cmd.split_whitespace().next().unwrap_or("unknown").to_string();
    }

    match parts.get(1) {
        Some(sub) if !sub.starts_with('"') && !sub.starts_with('\'') && !sub.starts_with('-') => {
            format!("ig {}", sub)
        }
        _ => "ig search".to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
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
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(5120), "5.0K");
        assert_eq!(format_bytes(1_048_576), "1.0M");
    }
}
