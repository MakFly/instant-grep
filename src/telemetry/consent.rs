//! Telemetry consent CLI flow.

use std::io::IsTerminal;

use super::{ConsentStatus, write_consent};

/// Resolve and persist a consent decision.
///
/// - `--yes` / `--no` are honored directly (and are mutually exclusive).
/// - With neither flag, prompt interactively when stdin is a TTY.
/// - With neither flag and no TTY, this is an error — telemetry must never
///   be enabled implicitly.
pub fn ask_consent(yes: bool, no: bool) -> anyhow::Result<ConsentStatus> {
    if yes && no {
        anyhow::bail!("--yes and --no are mutually exclusive");
    }
    if yes {
        write_consent(true)?;
        return Ok(ConsentStatus::Granted);
    }
    if no {
        write_consent(false)?;
        return Ok(ConsentStatus::Denied);
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("non-interactive session: pass --yes or --no to `ig telemetry consent`");
    }
    println!("ig telemetry is opt-in and off by default.");
    println!();
    println!("If enabled it sends anonymous aggregate counts only — no file");
    println!("paths, no file contents, no usernames, no cwd. The public build");
    println!("ships no endpoint, so enabling it is a no-op unless ig was built");
    println!("from source with IG_TELEMETRY_URL set.");
    println!();
    print!("Enable telemetry? [y/N] ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    write_consent(answer)?;
    Ok(if answer {
        ConsentStatus::Granted
    } else {
        ConsentStatus::Denied
    })
}
