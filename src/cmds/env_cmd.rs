//! `ig env [pattern]` — dump environment variables with sensitive value masking.
//!
//! Displays all environment variables sorted alphabetically.
//! If a pattern is provided, filters by variable name (case-insensitive).
//! Values of sensitive variables (KEY, SECRET, TOKEN, etc.) are masked.

use anyhow::Result;

/// Keywords that indicate a sensitive environment variable.
const SENSITIVE_KEYWORDS: &[&str] = &[
    "KEY", "SECRET", "TOKEN", "PASSWORD", "PASS",
    "API_KEY", "PRIVATE", "CREDENTIAL", "AUTH",
];

/// Run the env command.
pub fn run(args: &[String]) -> Result<i32> {
    let pattern = args.first().map(|s| s.to_lowercase());

    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));

    // Filter by pattern if provided
    if let Some(ref pat) = pattern {
        vars.retain(|(key, _)| key.to_lowercase().contains(pat));
    }

    for (key, value) in &vars {
        let display_value = if is_sensitive(key) {
            "****".to_string()
        } else {
            value.clone()
        };
        println!("{}={}", key, display_value);
    }

    if vars.is_empty() {
        if let Some(pat) = pattern {
            println!("No environment variables matching '{}'", pat);
        }
    }

    Ok(0)
}

/// Check if a variable name contains any sensitive keywords.
fn is_sensitive(key: &str) -> bool {
    let upper = key.to_uppercase();
    SENSITIVE_KEYWORDS.iter().any(|kw| upper.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_detection() {
        assert!(is_sensitive("AWS_SECRET_KEY"));
        assert!(is_sensitive("api_key"));
        assert!(is_sensitive("DATABASE_PASSWORD"));
        assert!(is_sensitive("GITHUB_TOKEN"));
        assert!(is_sensitive("PRIVATE_KEY"));
        assert!(is_sensitive("AUTH_HEADER"));
        assert!(!is_sensitive("HOME"));
        assert!(!is_sensitive("PATH"));
        assert!(!is_sensitive("EDITOR"));
    }
}
