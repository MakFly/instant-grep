//! `ig hook-audit` — print a table of installed-hook drift + a histogram of
//! permission/rewrite verdicts observed in the SQLite tracking DB.
//!
//! The verdict histogram is best-effort: it groups command rows by exit_code,
//! since the rewrite protocol stores its decision there
//! (0=passthrough, 1=ask/rewrite, 2=deny, 3=default-unfamiliar).

use anyhow::Result;
use std::collections::BTreeMap;

use super::integrity::{HookSignature, verify_installed};

/// Run the audit. `since_days` controls the verdict-histogram window.
/// When `json` is true, emit machine-readable JSON instead of a human table.
pub fn run_audit(since_days: u32, json: bool) -> Result<()> {
    let sigs = verify_installed();
    let verdicts = read_verdicts(since_days).unwrap_or_default();

    if json {
        print_json(&sigs, &verdicts, since_days);
    } else {
        print_human(&sigs, &verdicts, since_days);
    }
    Ok(())
}

/// Read exit-code histogram from the SQLite tracking DB for the last
/// `since_days` days. Returns `None` if the DB cannot be opened (e.g. no
/// rusqlite path resolvable).
fn read_verdicts(since_days: u32) -> Option<BTreeMap<i32, u64>> {
    use crate::analytics::TrackingDb;
    let db = TrackingDb::open().ok()?;
    let cutoff = now_secs().saturating_sub(since_days as u64 * 86_400) as i64;
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT exit_code, COUNT(*) FROM commands \
             WHERE timestamp >= ?1 AND exit_code IS NOT NULL \
             GROUP BY exit_code",
        )
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![cutoff], |row| {
            let code: i32 = row.get(0)?;
            let n: i64 = row.get(1)?;
            Ok((code, n as u64))
        })
        .ok()?;
    let mut out = BTreeMap::new();
    for r in rows.flatten() {
        out.insert(r.0, r.1);
    }
    Some(out)
}

fn verdict_label(code: i32) -> &'static str {
    match code {
        0 => "passthrough",
        1 => "ask/rewrite",
        2 => "deny",
        3 => "default-unfamiliar",
        _ => "other",
    }
}

fn print_human(sigs: &[HookSignature], verdicts: &BTreeMap<i32, u64>, since_days: u32) {
    println!("Hook integrity:");
    println!(
        "  {:<48}  {:<10}  {:<10}  STATUS",
        "installed_path", "expected", "actual"
    );
    if sigs.is_empty() {
        println!("  (no installed hooks discovered)");
    }
    for s in sigs {
        let status = if s.is_missing() {
            "MISSING"
        } else if s.is_drift() {
            "DRIFT"
        } else {
            "OK"
        };
        let exp = &s.expected_sha256[..8.min(s.expected_sha256.len())];
        let act = s
            .actual_sha256
            .as_deref()
            .map(|h| &h[..8.min(h.len())])
            .unwrap_or("--");
        println!(
            "  {:<48}  {:<10}  {:<10}  {}",
            s.installed_path.display(),
            exp,
            act,
            status,
        );
    }
    println!();
    println!("Verdict histogram (last {} days):", since_days);
    if verdicts.is_empty() {
        println!("  (no tracked commands with exit codes)");
    } else {
        for (code, n) in verdicts {
            println!("  exit {:>2}  {:<22}  {:>6}", code, verdict_label(*code), n);
        }
    }
}

fn print_json(sigs: &[HookSignature], verdicts: &BTreeMap<i32, u64>, since_days: u32) {
    // Hand-rolled JSON to avoid serde derive on HookSignature.
    let mut drift = Vec::with_capacity(sigs.len());
    for s in sigs {
        let status = if s.is_missing() {
            "MISSING"
        } else if s.is_drift() {
            "DRIFT"
        } else {
            "OK"
        };
        drift.push(serde_json::json!({
            "path": s.installed_path.display().to_string(),
            "expected": s.expected_sha256,
            "actual": s.actual_sha256,
            "status": status,
        }));
    }
    let mut verdict_map = serde_json::Map::new();
    for (code, n) in verdicts {
        verdict_map.insert(verdict_label(*code).into(), serde_json::json!(n));
    }
    let payload = serde_json::json!({
        "since_days": since_days,
        "drift": drift,
        "verdicts_last_n_days": verdict_map,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
    );
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
