use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Event, RecursiveMode, Watcher};

use crate::index::writer;
use crate::walk::DEFAULT_MAX_FILE_SIZE;

/// Watch a directory for file changes and rebuild the index when changes are detected.
/// Debounces rapid changes to avoid excessive rebuilds.
pub fn watch_and_rebuild(root: &Path, use_default_excludes: bool) -> Result<()> {
    let root = root.canonicalize().context("canonicalize root")?;

    // Initial build
    eprintln!("Building initial index...");
    let meta = writer::build_index(&root, use_default_excludes, DEFAULT_MAX_FILE_SIZE)?;
    eprintln!(
        "Indexed {} files, {} trigrams. Watching for changes...",
        meta.file_count, meta.ngram_count
    );

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let dominated_by_ig = event
                .paths
                .iter()
                .all(|p| p.to_string_lossy().contains(".ig/"));
            if !dominated_by_ig {
                let _ = tx.send(event);
            }
        }
    })
    .context("create file watcher")?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .context("watch directory")?;

    let debounce = Duration::from_millis(500);
    let mut last_rebuild = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(_event) => {
                // Debounce: wait for changes to settle
                while rx.recv_timeout(debounce).is_ok() {}

                if last_rebuild.elapsed() > Duration::from_secs(1) {
                    let start = Instant::now();
                    match writer::build_index(&root, use_default_excludes, DEFAULT_MAX_FILE_SIZE) {
                        Ok(meta) => {
                            eprintln!(
                                "Rebuilt: {} files, {} trigrams in {:.0}ms",
                                meta.file_count,
                                meta.ngram_count,
                                start.elapsed().as_secs_f64() * 1000.0,
                            );
                        }
                        Err(e) => {
                            eprintln!("Rebuild error: {}", e);
                        }
                    }
                    last_rebuild = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No changes, keep watching
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    Ok(())
}
