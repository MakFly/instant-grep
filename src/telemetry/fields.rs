//! Telemetry payload assembly.
//!
//! Strictly anonymous: version, OS/arch, aggregate command counts, parse
//! outcomes, installed-agent ids. No paths, no file contents, no usernames,
//! no cwd.

use serde_json::{Value, json};

use crate::analytics::sqlite::TrackingDb;

/// Build the anonymous telemetry payload.
pub fn assemble() -> Value {
    let db = TrackingDb::open().ok();

    let (tier_24h, top_24h) = match &db {
        Some(d) => match d.get_summary(Some(1), None) {
            Ok(s) => (
                s.count,
                s.top_commands
                    .iter()
                    .map(|(c, n)| json!([c, n]))
                    .collect::<Vec<_>>(),
            ),
            Err(_) => (0, Vec::new()),
        },
        None => (0, Vec::new()),
    };

    let total_rows = db
        .as_ref()
        .and_then(|d| d.get_summary(None, None).ok())
        .map(|s| s.count)
        .unwrap_or(0);

    let outcomes = db
        .as_ref()
        .map(parse_outcome_breakdown)
        .unwrap_or_else(empty_outcomes);

    json!({
        "ig_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tier_24h_runs": tier_24h,
        "top_commands_24h": top_24h,
        "parse_outcome_breakdown_24h": outcomes,
        "tracking_db_rows": total_rows,
        "agents_installed": installed_agents(),
        "consent_version": consent_version(),
    })
}

fn empty_outcomes() -> Value {
    json!({ "full": 0, "partial": 0, "passthrough": 0, "error": 0 })
}

fn parse_outcome_breakdown(db: &TrackingDb) -> Value {
    let cutoff = now_secs().saturating_sub(86_400) as i64;
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT COALESCE(parse_outcome,'unknown'), COUNT(*) FROM commands \
         WHERE timestamp >= ?1 GROUP BY parse_outcome",
    ) && let Ok(rows) = stmt.query_map([cutoff], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    }) {
        for row in rows.flatten() {
            counts.insert(row.0, row.1);
        }
    }
    json!({
        "full": counts.get("full").copied().unwrap_or(0),
        "partial": counts.get("partial").copied().unwrap_or(0),
        "passthrough": counts.get("passthrough").copied().unwrap_or(0),
        "error": counts.get("error").copied().unwrap_or(0),
    })
}

fn installed_agents() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    crate::setup::agents::all()
        .iter()
        .filter(|a| a.detect(&home))
        .map(|a| a.id().to_string())
        .collect()
}

fn consent_version() -> Value {
    super::data_dir()
        .map(|d| d.join("telemetry-consent.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("version").cloned())
        .unwrap_or(Value::Null)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
