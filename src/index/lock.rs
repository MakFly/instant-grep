//! Inter-process build lock for an index directory.
//!
//! Every `ig` invocation is a one-shot process, so two concurrent commands
//! (e.g. `ig index` in two terminals, or a search-triggered rebuild racing an
//! explicit index) can otherwise interleave their segment/publish writes and
//! leave a torn index. The lock serializes builders per `ig_dir`; readers are
//! never blocked (they mmap whatever was last published atomically).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const LOCK_FILE: &str = "index.lock";
/// How long a second builder waits before giving up. Real builds finish in
/// seconds (kweli: 1.5s, 3.5k-file monorepo: 4.7s); a waiter that obtains the
/// lock after this finds the index fresh and returns via the up-to-date path.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
/// A lock older than this is presumed dead even if its PID is alive (PID
/// recycling after reboot): no build legitimately takes 10 minutes.
const STALE_AGE: Duration = Duration::from_secs(600);
/// A lock whose PID can't be read yet is only stolen after this grace period
/// — the holder may be between `create_new` and writing its PID.
const PID_WRITE_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

/// RAII guard: holding an `IndexLock` means this process is the only index
/// builder for that directory. Dropping it (including on `?` unwind paths)
/// releases the lock.
pub struct IndexLock {
    path: PathBuf,
}

impl IndexLock {
    pub fn acquire(ig_dir: &Path) -> Result<Self> {
        fs::create_dir_all(ig_dir)
            .with_context(|| format!("create index dir {}", ig_dir.display()))?;
        let path = ig_dir.join(LOCK_FILE);
        let deadline = Instant::now() + ACQUIRE_TIMEOUT;

        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if holder_is_stale(&path) {
                        // Steal: remove and loop back to create_new. If two
                        // waiters race here, create_new picks a single winner.
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        let holder = fs::read_to_string(&path).unwrap_or_default();
                        bail!(
                            "index build already in progress (pid {}) — retry later or remove {}",
                            holder.trim(),
                            path.display()
                        );
                    }
                    std::thread::sleep(POLL);
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("create {}", path.display()));
                }
            }
        }
    }
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn holder_is_stale(path: &Path) -> bool {
    let age = match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(modified) => modified.elapsed().unwrap_or(Duration::ZERO),
        // Lock vanished (holder finished) or unreadable — let the caller
        // retry create_new rather than steal.
        Err(_) => return false,
    };
    if age > STALE_AGE {
        return true;
    }
    match fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    {
        // SAFETY: kill(pid, 0) sends no signal — it only reports whether the
        // process exists (0) or not (-1/ESRCH). No memory access involved.
        Some(pid) => (unsafe { libc::kill(pid, 0) }) != 0,
        // No PID yet: holder may not have written it — steal only past grace.
        None => age > PID_WRITE_GRACE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_is_exclusive_then_released_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = IndexLock::acquire(tmp.path()).unwrap();
        assert!(tmp.path().join(LOCK_FILE).exists());

        // A second acquire in another thread must block until we drop.
        let dir = tmp.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let _second = IndexLock::acquire(&dir).unwrap();
            start.elapsed()
        });
        std::thread::sleep(Duration::from_millis(300));
        drop(lock);
        let waited = handle.join().unwrap();
        assert!(
            waited >= Duration::from_millis(250),
            "second acquire should have waited, waited {:?}",
            waited
        );
        assert!(!tmp.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn stale_dead_pid_is_stolen() {
        let tmp = tempfile::tempdir().unwrap();
        // i32::MAX is far above any real PID on macOS/Linux → kill() == ESRCH.
        fs::write(tmp.path().join(LOCK_FILE), format!("{}\n", i32::MAX)).unwrap();
        let start = Instant::now();
        let _lock = IndexLock::acquire(tmp.path()).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "dead-pid lock should be stolen immediately"
        );
    }

    #[test]
    fn live_pid_lock_makes_acquire_wait() {
        let tmp = tempfile::tempdir().unwrap();
        // Our own PID is alive and the file is fresh → not stale.
        fs::write(
            tmp.path().join(LOCK_FILE),
            format!("{}\n", std::process::id()),
        )
        .unwrap();
        let dir = tmp.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let r = IndexLock::acquire(&dir);
            (start.elapsed(), r.is_ok())
        });
        std::thread::sleep(Duration::from_millis(400));
        // Holder "finishes": remove the lock; the waiter must then acquire.
        fs::remove_file(tmp.path().join(LOCK_FILE)).unwrap();
        let (waited, ok) = handle.join().unwrap();
        assert!(ok);
        assert!(waited >= Duration::from_millis(350));
    }
}
