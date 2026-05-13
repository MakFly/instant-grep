# SPEC — IDE-Tracker for the `ig` daemon

Status: **draft, v1**
Target: `ig` v1.20
Owner: kev
Date: 2026-05-13

---

## 1. Why

Today the `ig` daemon only learns about a project when the user explicitly searches in it
(or runs `ig warm`). On a machine with 28+ indexed projects, only **1 stays warm in the
daemon's LRU** at any given time — the cold-start hit of 7 s on the next project is felt
on every context switch.

Cursor (Anysphere) solves the same problem with `cursor-retrieval` + `crepectl`: a
file-system watcher service hooks `onDidChangeWorkspaceFolders`, re-initialises a grep
client per workspace, and keeps the indexes warm in the background. Confirmed by
reverse-engineering `cursor-retrieval/dist/main.js`:

```
class CFe {
  this._crepectlPath = this._getCrepectlPath();
  this.initializeGrepClient("activate");
  this._disposables.push(
    ca.workspace.onDidChangeWorkspaceFolders(() => {
      Wo.info("Workspace folders changed, re-initializing GrepClient...");
      this.disposeGrepClient();
      this.resetTrackedStateSnap…
    })
  );
}
```

`crepectl` (Mach-O ARM64 binary in
`/Applications/Cursor.app/Contents/Resources/app/resources/helpers/crepectl`) is
effectively **the same engine as `ig`**:

- Internal crate `crates/crepe/src/{ngrams,disk_index,filter,spillable_index}.rs`
- Same output layout (`index.bin`, `metadata.json`, `postings.bin`)
- Filter modes: `no-filter`, `hidden-file-filter`, `ripgrep-default-filter`
- Indexes a **git commit** (`crepectl build -w <worktree> -c <commit>`)

The differentiator is **not the indexer**, it is the **tracker** that drives the
indexer. This spec adds that tracker to `ig`, locally and without embeddings.

## 2. Goals & non-goals

**Goals**
- Daemon detects the projects the user is currently working in (via Cursor, VS Code,
  Claude Code state files) and keeps them warm proactively.
- Daemon optionally pre-warms the *files* the user is touching (Cursor open tabs,
  Claude Code recent `Read` events) so the next search hits a hot mmap.
- `ig projects list` shows the source of each warm tenant (`ide-cursor`,
  `ide-claude`, `ide-vscode`, `search`).
- Zero new CLI; the existing daemon observability surfaces are enough.

**Non-goals (this spec)**
- No embeddings, no semantic retrieval. (Separate spec.)
- No cloud upload, no telemetry.
- No new index format. We reuse `ig`'s sparse-trigram pipeline as-is.
- No hook into Cursor or VS Code extension APIs. Pure read-only access to their
  on-disk state files.

## 3. Sources of truth (read-only)

v1 ships with **Claude Code as the sole source** — that matches the maintainer's
real workflow today. Cursor and VS Code are dropped from v1 (no SQLite parsing,
no `state.vscdb` access, no `storage.json` polling) so the tracker stays tight
and trivially observable. Adding them later is a 1-file addition behind a flag.

| Source | Path | Contents | Format |
|---|---|---|---|
| Claude Code project dirs | `~/.claude/projects/<encoded-cwd>/` | mtime ≈ last activity for that project | filesystem |
| Claude Code recent `Read` events | `~/.claude/projects/<encoded>/<sessionId>.jsonl` | `tool_use` entries where `name == "Read"` | JSONL |

Path encoding: Claude Code stores `<cwd>` with `/` replaced by `-` and a leading
`-`. Example: `/Users/kev/Documents/lab/sandbox/instant-grep` →
`-Users-kev-Documents-lab-sandbox-instant-grep`. We invert this in
`decode_claude_project_dir()`.

On Linux the path is the same: `~/.claude/projects/`. No OS-specific code path.

## 4. Architecture

```
╔══════════════════════════ ig daemon ══════════════════════════╗
║                                                               ║
║  ┌─────────────────────────────────────────────────────────┐  ║
║  │  ide_tracker::claude_source (new module)                │  ║
║  │  single thread, poll interval 10 s                      │  ║
║  │                                                         │  ║
║  │  1. enumerate ~/.claude/projects/ subdirs               │  ║
║  │  2. for each subdir mtime within active_window (5 min): │  ║
║  │     - decode dir name → real cwd                        │  ║
║  │     - read newest *.jsonl (tail ~200 lines)             │  ║
║  │     - extract tool_use.input.file_path where Read       │  ║
║  │  3. emit IdeSignal { root, hot_files, ClaudeCode }      │  ║
║  └────────────────────────┬────────────────────────────────┘  ║
║                           │ mpsc                              ║
║                           ▼                                   ║
║  ┌─────────────────────────────────────────────────────────┐  ║
║  │  GlobalState::record_ide_signal(sig)                    │  ║
║  │   • if root not in LRU → warm_tenant(root) (background) │  ║
║  │   • if root in LRU      → bump tenant.last_seen         │  ║
║  │   • optional: pre-mmap top-N hot_files into tenant      │  ║
║  │   • respect LRU max_tenants (8) + memory governor       │  ║
║  └─────────────────────────────────────────────────────────┘  ║
║                                                               ║
║  ┌─────────────────────────────────────────────────────────┐  ║
║  │  Observability (no new CLI)                             │  ║
║  │   • `ig projects list` gains columns `source` + `hot`   │  ║
║  │   • `ig daemon status` exposes ide_signals/s counter    │  ║
║  │   • daemon.log: one line per warm event with source     │  ║
║  └─────────────────────────────────────────────────────────┘  ║
╚═══════════════════════════════════════════════════════════════╝
```

Legend:
- `IdeSource` enum: starts with `ClaudeCode` only. `Cursor` and `VSCode`
  variants are reserved for v2 — not implemented in v1.
- Hot files = the file paths of the most recent `Read` `tool_use` entries in
  the active session's JSONL, capped at 20.

## 5. Module contracts

### 5.1 `src/daemon/ide_tracker.rs` (new, ~250 LOC)

```rust
pub enum IdeSource { ClaudeCode }   // v2: + Cursor + VSCode

pub struct IdeSignal {
    pub root: PathBuf,            // canonicalised project root
    pub hot_files: Vec<PathBuf>,  // ≤ 20 most-recent files in this root
    pub source: IdeSource,
    pub last_seen: SystemTime,
}

/// Spawns the Claude Code poller thread. Returns a receiver the daemon
/// consumes. If `~/.claude/projects/` doesn't exist, the thread idles and
/// emits nothing — no error.
pub fn spawn_tracker(poll_interval: Duration) -> Receiver<IdeSignal>;
```

Implementation notes:

- Enumerate `~/.claude/projects/` once per poll cycle. Only directories whose
  mtime is within `active_window` (default 5 min) are candidates.
- For each candidate, pick the most-recently-modified `*.jsonl`. Tail it
  (last ~200 lines, or last 64 KiB if larger) via streaming
  `serde_json::Deserializer` — never load the full file. Extract
  `tool_use.input.file_path` entries where `name == "Read"`.
- Path encoding: the dir name is the cwd with `/` replaced by `-` and a
  leading `-`. Inverse:
  `dir_name.trim_start_matches('-').replace('-', '/')` → prepend `/` for the
  absolute path.
- Per-cycle dedup by `(root, hash(hot_files))` so the channel only fires when
  something actually changed since the previous tick.

### 5.2 `src/daemon.rs` (extend, ~80 LOC)

Add:

```rust
struct GlobalState {
    // existing fields…
    ide_signal_count: AtomicU64,  // for status output
}

impl GlobalState {
    /// Called from the dedicated thread that drains the tracker receiver.
    /// Lightweight: a single hashmap lookup + optional spawn.
    pub fn record_ide_signal(self: &Arc<Self>, sig: IdeSignal) {
        // 1. canonicalise root, check excludes
        // 2. if not in tenants → spawn warm_tenant_async(root)
        // 3. if in tenants → bump last_seen
        // 4. (later) mark hot_files for pre-mmap when capacity allows
        self.ide_signal_count.fetch_add(1, Ordering::Relaxed);
    }
}
```

Hook the tracker into `start_daemon` so a fresh daemon picks it up on boot.

### 5.3 `src/main.rs` (extend `ig projects list`, ~15 LOC)

New columns:

```
ROOT                                     LAST_SEEN  SOURCE        HOT
/Users/kev/.../tilvest/distribution-app   12s ago    ide-cursor    5
/Users/kev/.../instant-grep               1m04s ago  ide-claude   12
/Users/kev/.../workflow-rev               4m12s ago  search        0
```

`source` is the source of the *most-recent* signal for that tenant. `hot` is
the size of the last hot-file list (capped at 20).

## 6. Config knobs

Add to `~/.config/ig/config.toml` under `[ide_tracker]`:

```toml
[ide_tracker]
enabled = true
poll_interval_secs = 10
active_window_secs = 300          # ignore Claude projects untouched > 5 min
max_hot_files_per_signal = 20
excluded_roots = []                # absolute paths to never auto-warm
sources = ["claude"]               # v2: + "cursor" + "vscode"
```

Env overrides: `IG_IDE_TRACKER_ENABLED=0`, `IG_IDE_TRACKER_POLL_MS=10000`.

## 7. Edge cases & failure modes

- **State file locked**: `state.vscdb` may be locked when Cursor is launching.
  Read-only mode handles this; on failure we just skip the cycle.
- **No git root for hot file**: a Claude Code `Read` event might be for a file
  outside any project (e.g. `/tmp/foo`). We use `find_root()` to resolve it; if
  it returns the file's parent, we skip emitting (don't warm random temp dirs).
- **TCC prompts on first read**: Cursor / Claude state files are inside the
  user's home, no TCC trigger. macOS Privacy is not affected.
- **Memory pressure**: the existing memory governor already evicts tenants. The
  tracker only signals "this root is active"; the governor decides whether the
  warm survives. No new memory policy.
- **IDE switched off**: poll silently returns no signals. No thrashing.

## 8. Testing

- **Unit**: parse a fixture `<sessionId>.jsonl` (committed under
  `tests/fixtures/claude-session.jsonl`) and assert the Read paths are
  extracted in correct order, deduped, capped at 20.
- **Unit**: `decode_claude_project_dir("-Users-kev-Documents-foo")` →
  `/Users/kev/Documents/foo`. Round-trip with `encode_claude_project_dir`.
- **Integration**: spawn the daemon with `IG_IDE_TRACKER_POLL_MS=500`, create a
  temp `~/.claude/projects/-tmp-XYZ/` with a synthetic JSONL, assert
  `ig projects list --json` shows the decoded root with `source=ide-claude`
  within 2 s.
- **Real test (per CLAUDE.md policy — Claude `-p` driven)**:
  ```bash
  # 1. Fresh daemon
  ig daemon uninstall && ig daemon install

  # 2. Launch a Claude Code one-shot in a project the daemon hasn't seen
  cd /tmp && git init test-ide-tracker && cd test-ide-tracker
  echo 'fn main() { println!("hello"); }' > main.rs
  claude -p "read main.rs and tell me what it does"

  # 3. Wait one poll cycle (≤ 10 s), then verify
  sleep 12
  ig projects list   # → /tmp/test-ide-tracker should appear, source=ide-claude
  ig daemon status   # → ide_signals counter > 0
  ```
  Must reproduce on a clean cache and must not require any explicit `ig`
  invocation against the test project before step 3.

## 9. Roll-out

1. Land the spec (this file).
2. Implement `ide_tracker` module behind a feature flag (`IG_IDE_TRACKER=1`).
3. Default-on after a week of self-dogfooding on this machine.
4. v1.20 release notes call out the new behaviour explicitly so users who
   prefer the old passive warm can flip the toggle off.

## 10. Out of scope (next specs, not this one)

- **Semantic retrieval (`ig --semantic` with real embeddings)**: separate spec.
- **Claude Code `PreToolUse` hook for pre-fetch**: separate spec.
- **`.igignore` file** to let users carve out paths from auto-warm: trivial
  extension once this lands.
- **`ig hot`** sub-command listing the cross-source hot-set: post-MVP polish.

---

## Appendix A — Cursor `crepectl` evidence

```
$ /Applications/Cursor.app/Contents/Resources/app/resources/helpers/crepectl build --help
Build a fresh index from a git commit

Usage: crepectl build [OPTIONS]

Options:
  -w, --worktree <WORKTREE>      Repository worktree path [default: .]
  -c, --commit <COMMIT>          Git commit SHA to index (defaults to HEAD)
  -C, --cache-path <CACHE_PATH>
  -f, --filter <FILTER>          [possible values: no-filter, hidden-file-filter,
                                  ripgrep-default-filter]
  -M, --memory-limit <MB>        [default: 9216]
```

Internal source paths (from binary strings):
```
crates/crepe/src/bin/crepectl.rs
crates/crepe/src/disk_index.rs
crates/crepe/src/filter.rs
crates/crepe/src/freq.rs
crates/crepe/src/git/mod.rs
crates/crepe/src/ngrams.rs
crates/crepe/src/spillable_index.rs
```

Same `index.bin` / `metadata.json` / `postings.bin` triplet that `ig` writes.

## Appendix B — Why Cursor sources are deferred to v2

The maintainer's day-to-day driver is Claude Code, not Cursor. Adding
`state.vscdb` parsing now would mean:

- Pulling in `rusqlite` (or `sqlx`) as a new dependency, ~1 MB of compile-time
  weight.
- Handling locking semantics during Cursor's writes.
- Tracking the open-tabs key schema across Cursor releases (it's already
  changed between `glass.fileTab.viewState/*` in older releases and the new
  AI Pane state in 3.x).

For zero immediate UX benefit (Claude Code drives the workflow). When the user
flips to Cursor as primary driver, we add a single `cursor_source` module
behind the same `IdeSignal` contract — no schema migration needed.

## Appendix C — Claude Code Read-event evidence

For each session JSONL in `~/.claude/projects/<encoded>/<sessionId>.jsonl`,
`Read` invocations are recoverable with:

```
jq -rc '..|objects|select(.type=="tool_use" and .name=="Read")|.input.file_path' \
  ~/.claude/projects/<encoded>/<sessionId>.jsonl
```

Confirmed on this Mac: the current session listed 8 distinct files Claude has
opened — exactly the files we'd want pre-mmaped for the next search.
