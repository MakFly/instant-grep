//! `ig economics` — Cost analysis: tokens saved × price per token.
//!
//! Reads the ig tracking history and estimates cost savings
//! based on Claude Sonnet pricing.

use std::time::SystemTime;

/// Run the economics analysis and print results.
pub fn run_economics(since_days: u32) {
    let entries = crate::tracking::read_history();

    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cutoff = now_secs.saturating_sub(u64::from(since_days) * 86400);

    let recent: Vec<_> = entries.iter().filter(|e| e.timestamp >= cutoff).collect();

    if recent.is_empty() {
        println!("ig Economics (last {} days)", since_days);
        println!("════════════════════════════════════════════");
        println!("No tracking data found.");
        println!();
        println!("ig automatically tracks savings when used.");
        println!("Run some ig commands and check again.");
        return;
    }

    let total_saved_bytes: u64 = recent.iter().map(|e| e.saved_bytes).sum();
    let total_original_bytes: u64 = recent.iter().map(|e| e.original_bytes).sum();
    let total_output_bytes: u64 = recent.iter().map(|e| e.output_bytes).sum();

    // Estimate tokens (1 token ≈ 4 chars ≈ 4 bytes for ASCII-heavy code)
    let tokens_saved = total_saved_bytes / 4;
    let tokens_original = total_original_bytes / 4;

    // Claude pricing (Sonnet 4): $3/M input, $15/M output
    // Most savings are on input (tool results going back to Claude)
    let input_cost_saved = (tokens_saved as f64 / 1_000_000.0) * 3.0;

    let savings_pct = if total_original_bytes > 0 {
        (total_saved_bytes as f64 / total_original_bytes as f64) * 100.0
    } else {
        0.0
    };

    println!("ig Economics (last {} days)", since_days);
    println!("════════════════════════════════════════════");
    println!("Commands tracked:  {}", recent.len());
    println!("Original output:   {}", format_bytes(total_original_bytes));
    println!("Compressed output: {}", format_bytes(total_output_bytes));
    println!(
        "Bytes saved:       {} ({:.0}%)",
        format_bytes(total_saved_bytes),
        savings_pct
    );
    println!();
    println!("Tokens saved:      ~{}", format_number(tokens_saved));
    println!("Tokens original:   ~{}", format_number(tokens_original));
    println!("Cost saved:        ~${:.2}", input_cost_saved);
    println!();
    println!("Based on Claude Sonnet pricing ($3/M input tokens).");
    println!("Actual savings depend on your plan and model.");
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1_024), "1.0 KB");
        assert_eq!(format_bytes(5_500), "5.4 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(5_767_168), "5.5 MB");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(1_500_000), "1.5M");
    }
}
