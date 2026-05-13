# SPEC — IDE-Tracker for the `ig` daemon

Status: **shipped, v1.1**
Target: `ig` v1.20
Owner: kev
Date: 2026-05-13 (v1), revised 2026-05-13 (v1.1 multi-provider)

## Changelog

| Version | Date | Change |
|---|---|---|
| v1   | 2026-05-13 | Claude Code only. Shipped in `fa97846`. |
| v1.1 | 2026-05-13 | Refactor in trait `IdeProvider`; add Codex CLI and opencode providers. Shipped in `2fe4346`. |

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

v1.1 ships with **three providers**, all read-only, all local, zero embedding
or cloud involvement. Each maps to one well-known agent's on-disk state.

| Provider | id | Path | Field of interest | Format |
|---|---|---|---|---|
| Claude Code | `claude-code` | `~/.claude/projects/<encoded>/<sid>.jsonl` | top-level `cwd` + `tool_use Read` events | JSONL |
| Codex CLI (OpenAI) | `codex` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `payload.cwd` in the `session_meta` event | JSONL |
| opencode (sst) | `opencode` | `~/.local/state/opencode/frecency.jsonl` | `{path, frequency, lastOpen}` per entry | JSONL |

Cursor app and VS Code state (`state.vscdb` SQLite) remain deferred to v2
(see Appendix B). The maintainer drives via Claude Code / Codex / opencode,
so adding a sqlite dep for zero immediate UX gain isn't justified.

Path encoding notes:

- **Claude Code** stores `<cwd>` with `/` replaced by `-` and a leading `-`.
  Example: `/Users/kev/Documents/foo` → `-Users-kev-Documents-foo`. The
  decoding is lossy for paths containing `-` (e.g. `kweli-project`); we
  resolve the ambiguity by reading the `cwd` field embedded in each
  JSONL entry, falling back to `decode_claude_project_dir()` only when no
  `cwd` was observed.
- **Codex** uses a date-partitioned tree. `recent_rollouts(N=16)` walks
  newest year → month → day → file and stops at 16 candidates so old
  archives don't blow the poll budget.
- **opencode** frecency tracks both files and directories under the same
  schema. `find_project_root()` walks each entry up to the nearest
  `.git/` or project marker, then buckets by root.

On Linux the paths are the same: `~/.claude/`, `~/.codex/`, `~/.local/state/`.
No OS-specific code path.

## 4. Architecture

```
╔═══════════════════════════════ ig daemon ══════════════════════════════╗
║                                                                        ║
║  ┌──────────────────────────────────────────────────────────────────┐ ║
║  │  ide_tracker — one thread, poll interval 10 s                    │ ║
║  │  Iterates `enabled_providers()` (filtered by                     │ ║
║  │  IG_IDE_TRACKER_PROVIDERS env)                                   │ ║
║  │                                                                  │ ║
║  │   ┌──────────────────┐  ┌──────────────────┐  ┌───────────────┐ │ ║
║  │   │ ClaudeCode-      │  │  Codex-          │  │ OpenCode-     │ │ ║
║  │   │ Provider         │  │  Provider        │  │ Provider      │ │ ║
║  │   │ ~/.claude/       │  │ ~/.codex/        │  │ ~/.local/state│ │ ║
║  │   │   projects/      │  │   sessions/      │  │   /opencode/  │ │ ║
║  │   │ → cwd + Read     │  │ → session_meta   │  │ frecency.jsonl│ │ ║
║  │   │   tool_use       │  │   .payload.cwd   │  │ → {path,      │ │ ║
║  │   │   events         │  │                  │  │   lastOpen}   │ │ ║
║  │   └─────────┬────────┘  └─────────┬────────┘  └───────┬───────┘ │ ║
║  │             │                     │                   │         │ ║
║  │             ▼                     ▼                   ▼         │ ║
║  │   ┌──────────────────────────────────────────────────────────┐  │ ║
║  │   │ dedup per (root, source) → mpsc Sender<IdeSignal>        │  │ ║
║  │   └──────────────────────────┬───────────────────────────────┘  │ ║
║  └──────────────────────────────┼──────────────────────────────────┘ ║
║                                 │ IdeSignal { root, hot_files,        ║
║                                 │   source: ClaudeCode|Codex|OpenCode,║
║                                 │   last_seen }                       ║
║                                 ▼                                     ║
║  ┌──────────────────────────────────────────────────────────────────┐ ║
║  │  GlobalState::record_ide_signal(sig)                             │ ║
║  │   • if root not in LRU → warm_tenant(root) (background)          │ ║
║  │   • if root in LRU     → bump tenant.last_seen                   │ ║
║  │   • last-signal-wins on `source` column when 2+ providers see    │ ║
║  │     the same project                                             │ ║
║  │   • respect LRU max_tenants (8) + memory governor                │ ║
║  └──────────────────────────────────────────────────────────────────┘ ║
║                                                                        ║
║  ┌──────────────────────────────────────────────────────────────────┐ ║
║  │  Observability (no new CLI)                                      │ ║
║  │   • boot log: `ide-tracker: active providers = [claude-code,…]`  │ ║
║  │   • `ig projects list` adds `source=…  hot=N` columns            │ ║
║  │   • daemon.log: signal-per-line with `source=ide-{claude|codex|  │ ║
║  │     opencode}`                                                   │ ║
║  └──────────────────────────────────────────────────────────────────┘ ║
╚════════════════════════════════════════════════════════════════════════╝
```

Legend:
- `IdeSource` enum: `ClaudeCode | Codex | OpenCode`. Each maps to a stable
  string (`ide-claude`, `ide-codex`, `ide-opencode`).
- Hot files = provider-specific (Claude Code → `tool_use Read` paths;
  opencode → recent files inside the project root; Codex → reserved for
  the rare tool_use shape — typically empty in v1.1). Capped at 20.
- "Last-signal-wins" on `source`: simple and conforms to the maintainer's
  intuition (whichever agent you used most recently is what `projects
  list` shows). The full signal stream is preserved in `daemon.log`.

## 5. Module contracts

### 5.1 `src/ide_tracker.rs` (~800 LOC for v1.1)

Single-file module exposing the trait and three providers. Kept monolithic
to make changes diffable and to avoid the `mod.rs` indirection at this
size; if the file grows past ~1200 LOC it should be split into
`src/ide_tracker/{mod, claude, codex, opencode}.rs`.

```rust
pub enum IdeSource { ClaudeCode, Codex, OpenCode }

pub struct IdeSignal {
    pub root: PathBuf,            // canonicalised project root
    pub hot_files: Vec<PathBuf>,  // ≤ 20 most-recent files in this root
    pub source: IdeSource,
    pub last_seen: SystemTime,
}

trait IdeProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn ide_source(&self) -> IdeSource;
    fn is_available(&self) -> bool;
    fn scan(&self, cutoff: SystemTime) -> Vec<IdeSignal>;
}

/// Spawns the tracker thread that iterates all available providers
/// every `poll_interval`. Returns a receiver the daemon consumes.
/// If no provider's state dir exists, the thread idles without
/// log spam.
pub fn spawn_tracker(poll_interval: Duration) -> Receiver<IdeSignal>;
```

Implementation notes (shared by all providers):

- Each provider's `scan()` is bounded: mtime filters, tail-byte reads
  (`JSONL_TAIL_BYTES = 64 KiB`), and provider-specific caps (`recent_rollouts(16)`
  for Codex). Total poll cost on this Mac: ~5-15 ms.
- The dedup key is `(root, source)`, not just `root`. The same project
  warmed by Claude AND opencode generates two separate signal streams,
  preserved in `daemon.log`. `record_ide_signal` then applies
  last-signal-wins on the `source` column.
- `canonical_or_skip()` does two things in one shot: canonicalise the
  candidate root and run `looks_like_project_root()` (rejects `~`, `/`,
  `/Users`, `/home`, and paths without a `.git/` or project marker).
- `looks_like_project_root()` and `find_project_root()` are duplicated
  inside the module rather than imported from `util::` — the v1.1 module
  is intentionally standalone so the daemon's existing utilities remain
  cleanly decoupled from a feature flag.

Per-provider specifics:

- **Claude Code** — source of truth is the `cwd` field inside the JSONL.
  Falls back to `decode_claude_project_dir()` only when no `cwd` is seen.
  The decode is lossy for paths containing `-` (Claude doesn't escape it),
  so the canonical step skips phantoms.
- **Codex** — date-tree walk; reads `payload.cwd` from `session_meta`. We
  walk newest year → month → day, accumulate up to `limit * 3 = 48`
  candidates, sort by mtime descending, truncate to 16.
- **opencode** — single file (`frecency.jsonl`), single pass. Each entry
  is a `(path, frequency, lastOpen)` triple; we bucket per project root
  via `find_project_root` and emit one signal per root.

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

New columns (tab-separated, `awk`/`cut` parseable):

```
/Users/kev/.../instant-grep                   last_seen=3s    source=ide-codex      hot=0
/Users/kev/.../tilvest/distribution-app-v2    last_seen=12s   source=ide-claude     hot=4
/private/tmp/demo-tracker                     last_seen=11s   source=ide-claude     hot=1
/Users/kev/.../kweli-project                  last_seen=2m    source=ide-opencode   hot=0
```

`source` is the source of the *most-recent* signal for that tenant
(`ide-claude` / `ide-codex` / `ide-opencode` / `search` when the project
was warmed by an explicit `ig` command rather than the tracker). `hot` is
the size of the last hot-file list, capped at 20.

## 6. Config knobs

```toml
[ide_tracker]
enabled = true
poll_interval_secs = 10
active_window_secs = 300            # ignore state-files untouched > 5 min
max_hot_files_per_signal = 20
excluded_roots = []                  # absolute paths to never auto-warm
providers = ["claude", "codex", "opencode"]
```

Env overrides:
- `IG_IDE_TRACKER_ENABLED=0` — master kill switch.
- `IG_IDE_TRACKER_POLL_MS=10000` — cadence override.
- `IG_IDE_TRACKER_PROVIDERS="claude,opencode"` — opt out of a provider
  without disabling the whole tracker. Accepted values: `claude`,
  `claude-code`, `codex`, `opencode`, `all` (default = all enabled).

## 7. Edge cases & failure modes

- **State file locked / missing fields**: every JSONL parse is best-effort.
  Malformed lines, unexpected schemas, and IO errors are silently skipped.
- **Codex `payload.cwd` absent**: the rollout is skipped (we don't try
  to infer the cwd from file paths).
- **opencode entry without a project root**: `find_project_root` returns
  `None` and the entry is dropped (don't warm `/tmp/foo` or arbitrary
  files).
- **TCC prompts on first read**: all state files are inside the user's
  home — no Documents/Desktop trigger.
- **Memory pressure**: the existing memory governor already evicts tenants. The
  tracker only signals "this root is active"; the governor decides whether the
  warm survives. No new memory policy.
- **IDE switched off**: poll silently returns no signals. No thrashing.

## 8. Testing

Per-provider unit tests live in `src/ide_tracker.rs` (8 tests as of v1.1):

- **shared**: `decode_round_trip_for_paths_without_dashes`,
  `decode_rejects_bad_input`, `looks_like_project_root_basic`,
  `collect_read_paths_dedupes_and_orders`.
- **Claude Code**: `claude_provider_extracts_cwd_from_jsonl_with_dashed_dir_name`
  proves the `cwd` field beats the lossy dir-name decode.
- **Codex**: `codex_session_meta_payload_cwd_is_extracted` parses a
  fixture rollout and asserts the `payload.cwd` is recovered.
- **opencode**: `opencode_frecency_buckets_paths_by_project_root`
  synthesises a `frecency.jsonl`, walks `find_project_root` and asserts
  per-root bucketing + stale-entry filtering.

Real test (CLAUDE.md policy, executed on this Mac on 2026-05-13):

```bash
# Setup
mkdir /tmp/demo-tracker && cd $_ && git init -q
echo 'fn main(){}' > main.rs && git add -A && git commit -qm init

# Provoke a signal for each provider
claude -p "read main.rs"                              # claude-code
echo '{"type":"session_meta","payload":{"cwd":"/Users/kev/.../instant-grep"}}' \
  > ~/.codex/sessions/2026/05/13/rollout-synth.jsonl   # codex (synth fixture)
echo '{"path":"/Users/kev/.../kweli-project/apps/api","lastOpen":'$(date +%s000)'}' \
  >> ~/.local/state/opencode/frecency.jsonl            # opencode

sleep 12 && ig projects list
# Observed:
#   /private/tmp/demo-tracker                  source=ide-claude     hot=1
#   /Users/kev/.../instant-grep                source=ide-codex      hot=0
#   /Users/kev/.../kweli-project               source=ide-claude     hot=0   (or ide-opencode pre-Claude poll)
```

Pass criterion: each of the three providers must contribute at least one
recorded signal in `daemon.log` within 30 s.

## 9. Roll-out

1. ~~Land the spec~~ (done: `20667a0`).
2. ~~Implement Claude Code tracker (v1)~~ (done: `fa97846`).
3. ~~Refactor to provider trait + add Codex + opencode (v1.1)~~ (done: `2fe4346`).
4. **Next**: self-dogfood for ≥ 1 week. Reassess defaults (`active_window`,
   `recent_rollouts(N)`, hot-files cap) based on real signal volume.
5. v1.20 release notes: explicit mention of the new behaviour + opt-out
   instructions for users who prefer the passive warm.

## 10. Out of scope (next specs, not this one)

- **Semantic retrieval (`ig --semantic` with real embeddings)**: separate spec.
- **Claude Code `PreToolUse` hook for pre-fetch**: separate spec.
- **Cursor app + VS Code state.vscdb sources**: deferred (sqlite dep weight
  vs current UX gain). See Appendix B.
- **`.igignore` file** to let users carve out paths from auto-warm: trivial
  extension once this lands.
- **`ig hot`** sub-command listing the cross-source hot-set: post-MVP polish.
- **Pre-mmap hot_files** into the tenant on signal: covered by the trait
  already (signals carry `hot_files`), but the warmer doesn't act on them
  yet. ~50 LOC follow-up.

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
