//! Fire-and-forget telemetry ping.
//!
//! Runs on a detached thread so it can never block — or fail — the
//! user-facing command. Any network error is swallowed.

/// Spawn a detached thread that POSTs `payload` to `url`.
///
/// Only ever called from [`super::maybe_ping`], which itself only runs when
/// an endpoint was compiled in and consent was granted. In a public build
/// this function is unreachable.
pub fn send_async(url: String, payload: serde_json::Value) {
    std::thread::spawn(move || {
        let _ = ureq::post(url.as_str())
            .header("Content-Type", "application/json")
            .send_json(&payload);
    });
}
