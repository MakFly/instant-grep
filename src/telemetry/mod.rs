//! Opt-in telemetry. Off by default.
//!
//! NO endpoint URL is compiled into the open-source binary — [`maybe_ping`]
//! is a hard no-op unless the crate was built with the `IG_TELEMETRY_URL`
//! environment variable set at compile time. Even in such a custom build it
//! only fires when the user has explicitly granted consent via
//! `ig telemetry consent --yes` and `IG_TELEMETRY_DISABLED` is unset.
//!
//! The payload (see [`fields`]) carries no PII: no paths, no usernames, no
//! cwd, no file contents — only anonymous aggregate counts.

pub mod consent;
pub mod fields;
pub mod ping;

use std::path::PathBuf;

use crate::cli::TelemetryOp;

/// Whether the user has been asked, and what they answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentStatus {
    NotAsked,
    Granted,
    Denied,
}

/// Compile-time endpoint. `None` in every public release build.
pub fn endpoint() -> Option<&'static str> {
    option_env!("IG_TELEMETRY_URL")
}

fn data_dir() -> Option<PathBuf> {
    crate::analytics::sqlite::data_dir()
}

fn consent_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("telemetry-consent.json"))
}

fn last_ping_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("telemetry-last-ping.timestamp"))
}

/// Read the persisted consent decision.
pub fn consent_status() -> ConsentStatus {
    let Some(p) = consent_path() else {
        return ConsentStatus::NotAsked;
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return ConsentStatus::NotAsked;
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => match v.get("answer").and_then(|a| a.as_bool()) {
            Some(true) => ConsentStatus::Granted,
            Some(false) => ConsentStatus::Denied,
            None => ConsentStatus::NotAsked,
        },
        Err(_) => ConsentStatus::NotAsked,
    }
}

/// Persist a consent decision to `telemetry-consent.json`.
pub(crate) fn write_consent(answer: bool) -> anyhow::Result<()> {
    let p = consent_path().ok_or_else(|| anyhow::anyhow!("no data dir available"))?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "given_at": now_secs(),
        "answer": answer,
        "version": env!("CARGO_PKG_VERSION"),
    });
    std::fs::write(&p, serde_json::to_string_pretty(&body)?)?;
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Last successful ping timestamp (unix secs), 0 if never pinged.
fn last_ping_secs() -> u64 {
    last_ping_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn touch_last_ping() {
    if let Some(p) = last_ping_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(p, now_secs().to_string());
    }
}

/// Fire a telemetry ping iff every gate is satisfied: an endpoint was
/// compiled in, `IG_TELEMETRY_DISABLED` is unset, consent is granted, and
/// the 23h rate-limit window has elapsed. Fully no-op in the public build.
pub fn maybe_ping() {
    if std::env::var_os("IG_TELEMETRY_DISABLED").is_some() {
        return;
    }
    let Some(url) = endpoint() else {
        return;
    };
    if consent_status() != ConsentStatus::Granted {
        return;
    }
    if now_secs().saturating_sub(last_ping_secs()) < 23 * 3600 {
        return;
    }
    let payload = fields::assemble();
    touch_last_ping();
    ping::send_async(url.to_string(), payload);
}

/// `ig telemetry <op>` dispatch.
pub fn run(op: TelemetryOp) -> anyhow::Result<()> {
    match op {
        TelemetryOp::Status => {
            let status = match consent_status() {
                ConsentStatus::NotAsked => "not asked",
                ConsentStatus::Granted => "granted",
                ConsentStatus::Denied => "denied",
            };
            let last = last_ping_secs();
            println!("consent:        {}", status);
            println!(
                "last ping:      {}",
                if last == 0 {
                    "never".to_string()
                } else {
                    format!("{} (unix epoch secs)", last)
                }
            );
            println!(
                "endpoint build: {}",
                if endpoint().is_some() {
                    "compiled in"
                } else {
                    "none — offline build, ping is a no-op"
                }
            );
            println!(
                "env kill:       {}",
                if std::env::var_os("IG_TELEMETRY_DISABLED").is_some() {
                    "IG_TELEMETRY_DISABLED set"
                } else {
                    "unset"
                }
            );
        }
        TelemetryOp::Consent { yes, no } => match consent::ask_consent(yes, no)? {
            ConsentStatus::Granted => {
                println!("Telemetry consent granted.");
                if endpoint().is_none() {
                    println!("(this build ships no endpoint — telemetry stays a no-op)");
                }
            }
            ConsentStatus::Denied => {
                println!("Telemetry consent denied. ig stays fully offline.")
            }
            ConsentStatus::NotAsked => {}
        },
        TelemetryOp::Test => {
            let payload = fields::assemble();
            println!("{}", serde_json::to_string_pretty(&payload)?);
            eprintln!("(dry-run: payload assembled above, not sent)");
        }
    }
    Ok(())
}
