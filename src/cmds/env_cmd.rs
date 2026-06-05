//! `ig env [pattern]` — dump environment variables with sensitive value masking.
//!
//! Displays all environment variables sorted alphabetically.
//! If a pattern is provided, filters by variable name (case-insensitive).
//! Values of sensitive variables (KEY, SECRET, TOKEN, etc.) are masked.

use anyhow::Result;

/// Keywords that indicate a sensitive environment variable.
const SENSITIVE_KEYWORDS: &[&str] = &[
    "KEY",
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASS",
    "API_KEY",
    "PRIVATE",
    "CREDENTIAL",
    "AUTH",
];

/// Variables dropped entirely — high-noise, zero value for agents.
const DROP_VARS: &[&str] = &[
    "LS_COLORS",
    "LSCOLORS",
    "TERMCAP",
    "_",
    "SHLVL",
    "OLDPWD",
    "TMPDIR",
    "LESS",
    "PAGER",
    "MANPATH",
    "INFOPATH",
    "FPATH",
    "COLORTERM",
    "TERM_PROGRAM",
    "TERM_SESSION_ID",
];

/// Prefixes of variables dropped entirely.
const DROP_PREFIXES: &[&str] = &[
    "XPC_",
    "__CF",
    "ITERM_",
    "CMUX_",
    "VSCODE_",
    "HOMEBREW_",
    "PYENV_",
    "RBENV_",
    "NVM_",
    "VOLTA_",
    "ANDROID_",
    "FLUTTER_",
    "LC_",
    "OTEL_",
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

    // Drop noise vars (unless user explicitly filtered)
    if pattern.is_none() {
        vars.retain(|(key, _)| {
            !DROP_VARS.contains(&key.as_str()) && !DROP_PREFIXES.iter().any(|p| key.starts_with(p))
        });
    }

    for (key, value) in &vars {
        let display_value = if is_sensitive(key) {
            "****".to_string()
        } else if key == "PATH" {
            truncate_path(value)
        } else if value.len() > 200 {
            format!("{}… ({} chars)", &value[..100], value.len())
        } else {
            value.clone()
        };
        println!("{}={}", key, display_value);
    }

    if vars.is_empty()
        && let Some(pat) = pattern
    {
        println!("No environment variables matching '{}'", pat);
    }

    Ok(0)
}

/// Truncate PATH to first 5 entries + count of remaining.
fn truncate_path(value: &str) -> String {
    let entries: Vec<&str> = value.split(':').collect();
    if entries.len() <= 5 {
        return value.to_string();
    }
    let shown: Vec<&str> = entries[..5].to_vec();
    format!("{} (+{} more)", shown.join(":"), entries.len() - 5)
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
