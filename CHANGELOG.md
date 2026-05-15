# Changelog

All notable changes to `instant-grep` are documented here. Format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions adhere to [SemVer](https://semver.org/).

## [1.20.2] — 2026-05-15

### Fixed — `ig hold begin` under daemon soft-RSS pressure

`ig hold begin <project>` no longer fails just because the global daemon is
above its soft RSS limit. Session holds are protective agent edit locks, not
search warmups, so they now activate the lightweight watcher path under soft
memory pressure and only abort when the daemon reaches the hard limit.

This fixes the Codex/Claude agent path where a failed hold caused the agent to
fall back to `rg` after seeing:

```text
daemon memory soft limit reached during warm project ... background activation paused
```

The memory reclaimer also evicts the final cached tenant now. Previously, a
single large reader could keep RSS above the soft limit forever while
`ig projects list` showed no active projects, causing every new project warm or
hold to be rejected in a loop.

## [1.20.1] — 2026-05-14

### Fixed — daemon deadlock on pre-v1.20 → v1.20 upgrades

A machine still on the pre-v1.20 shim+backend layout could end up with the
global daemon **permanently dead** after `install.sh` / `ig update`, with
`ig daemon status` reporting "not running" and the systemd unit exiting
`0` (so `Restart=on-failure` never kicked in). Root cause was a three-bug
cascade, reproduced on a real Ubuntu host:

1. **`purge_legacy_ig_rust_daemons()` refused to reap a legacy `ig-rust`
   daemon that owned the canonical socket.** The `owns_socket()` guard
   assumed "owns `daemon.sock` ⇒ it's the real daemon". False: pre-v1.20
   and v1.20+ share the same cache layout, so a legacy `ig-rust` *always*
   owns the canonical socket. The guard made a wedged `ig-rust` immortal.
   It is now applied only to ambiguous `target/{debug,release}/ig` test
   orphans; a legacy `ig-rust` is always reaped, since replacing it is the
   whole point of the migration.
2. **`start_daemon()` acquired the start lock before purging.** A legacy
   `ig-rust` holds an `flock` on `daemon.lock` for its entire life, so
   `acquire_daemon_start_lock()` returned `None`, `start_daemon()` bailed
   with a misleading "already running", and `purge_legacy_per_project_daemons()`
   — three lines further down — never ran. The purge now runs *before*
   the lock is acquired, behind a fast-path `is_daemon_available()` check.
3. **The reaper only sent `SIGTERM`.** A daemon wedged after a hard-RSS
   hit ("shutting down" on every query but never exiting) ignores
   `SIGTERM`. The reaper now escalates to `SIGKILL` after a 3 s grace
   window, waits for the process to actually exit (releasing the `flock`),
   and clears the stale `daemon.sock` / `daemon.pid` it left behind.

### Fixed — `ig update` no longer deletes the binary it just installed

In the pre-v1.20 shim layout, `ig update` runs from inside
`~/.local/share/ig/bin/ig-rust`, so `current_exe()` — the install target —
*is* a legacy `ig-rust` path. `clean_legacy_backend()` then deleted that
exact file immediately after `atomic_install()` wrote to it, leaving the
surviving C shim pointing at nothing. The sweep now receives the install
target and excludes it (`legacy_backend_candidates()`), with regression
tests.

## [1.20.0] — 2026-05-13

### Changed (breaking, packaging only) — collapsed to a single Rust binary

Pre-v1.20 instant-grep shipped two artefacts per arch: a ~35 KB C shim at `~/.local/bin/ig` that `execve`'d a hidden Rust backend at `~/.local/share/ig/bin/ig-rust`. The shim's purpose was a sub-2 ms cold start on hot subcommands; that savings has been irrelevant ever since the global daemon socket round-trip (1-5 ms) became the steady-state hot path, and the dual-binary layout cost a second build toolchain in CI, ~320 LOC of glue across `install.sh` / `update.rs` / `daemon.rs`, and a recurring class of "shim can't find backend" bugs.

v1.20 ships **one Rust binary** per platform — `ig-linux-x86_64`, `ig-linux-aarch64`, `ig-macos-x86_64`, `ig-macos-aarch64`. No more `*-rust` artifacts. The single binary lives at `~/.local/bin/ig` (or wherever the existing `ig` was found on upgrade).

User-facing impact: **none.** `ig` and `ig <subcommand>` work exactly as before. Speed is unchanged on warm paths (the daemon round-trip dominates) and within ~1 ms on cold paths (the `fork()`+`execve()` overhead of the old C shim is gone, mostly offsetting Rust's slightly longer cold start).

Migration is automatic for existing users: both `install.sh` and `ig update` detect a stale `ig-rust` at any of `~/.local/share/ig/bin/`, `~/.local/bin/`, `~/.cargo/bin/`, `/usr/local/share/ig/bin/`, `/usr/local/bin/`, `/opt/homebrew/share/ig/bin/` and remove it (plus the now-empty share dirs). No action needed beyond the next update.

### Removed
- `shim/` directory: `ig.c` (445 LOC) + `parse.h` + the 6 `test_*.c` regression files. ~150 LOC of `release.yml` C-cross-compilation glue.
- `*-rust` artefacts from GitHub Releases. Pre-v1.20 releases keep theirs.
- `resolve_install_targets()` + `locate_shim_in_path()` in `src/update.rs`: ~50 LOC. Replaced by a single `current_exe()` + `atomic_install()`.

### Added
- `clean_legacy_backend()` in `src/update.rs`: idempotent sweep of all known legacy `ig-rust` paths on every update.

### Internal
- `detect_artifact()` now returns `String` instead of `(String, String)`.
- `install.sh` reduced from ~190 LOC to ~145 LOC.
- `release.yml` matrix simplified: no more `cc` / `aarch64-linux-gnu-gcc` install steps, no more shim build step.

## [1.19.13] — 2026-05-13

### Fixed — CI green on Rust 1.95.0

- `clippy::unnecessary_sort_by` (new in 1.95) tripped on
  `src/ide_tracker.rs:450` (`out.sort_by(|a, b| b.1.cmp(&a.1))`). Switched
  to `sort_by_key(|e| std::cmp::Reverse(e.1))`.
- `cargo fmt` reformatting on 1.95.0 nightly toolchain (split `let-else`
  one-liners that older rustfmt accepted, joined a short `||` chain).
  Pure formatting, no behavioural change.
- v1.19.12 binaries on GitHub Releases remain functional — only the
  CI workflow on `main` was red. v1.19.13 simply gives `main` a green
  pipeline again.

## [1.19.12] — 2026-05-13

### Added — IDE tracker, multi-provider (Claude Code / Codex CLI / opencode)

The daemon now learns proactively which projects you're working on by reading the on-disk state of three popular AI-coding agents — no embeddings, no cloud, no IDE extension required. Reverse-engineering [`cursor-retrieval`](https://github.com/getcursor/cursor) confirmed Anysphere's stack relies on a sibling Rust binary (`crepectl`) with a near-identical n-gram pipeline; the differentiator was their tracker, not their indexer. This release closes the gap.

- **Three providers** parsed locally and read-only:
  - `claude-code` → `~/.claude/projects/<encoded>/<sessionId>.jsonl` (top-level `cwd` field + `tool_use Read` events).
  - `codex` → `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (`payload.cwd` from `session_meta`).
  - `opencode` → `~/.local/state/opencode/frecency.jsonl` (`{path, lastOpen}` bucketed per project root).
- Poll cadence: 10 s. Per-cycle dedup keyed on `(root, source)` so the same project warmed by two agents emits two distinct signal streams (visible in `daemon.log`).
- `ig projects list` gains `source=ide-…  hot=N` columns. Last-signal-wins on the `source` column when multiple providers see the same root.
- Boot log: `ide-tracker: active providers = [claude-code, codex, opencode]`. Lets you verify at a glance which agents the daemon is watching.
- Env knobs: `IG_IDE_TRACKER_PROVIDERS="claude,codex,opencode"` (default = all), `IG_IDE_TRACKER_ENABLED=0` (master kill switch), `IG_IDE_TRACKER_POLL_MS=10000` (cadence override).
- Spec: [`docs/specs/SPEC-ide-tracker.md`](docs/specs/SPEC-ide-tracker.md).
- Cursor app + VS Code (`state.vscdb`) sources are explicitly deferred to v2 — the sqlite weight isn't justified while the maintainer's daily drivers are Claude/Codex/opencode.

### Fixed — daemon lifecycle bulletproof on macOS

- `launchctl load/unload` is deprecated since Catalina and returns `Input/output error` on Sonoma+. Switched `install_launchd` to the modern `bootstrap`/`bootout` API against `gui/<uid>`, with a short retry-backoff for the EIO case where launchd hasn't fully torn down the previous job yet. Fresh installs that used to silently leave the daemon down now reliably start it.
- `install_launchd` is now **idempotent and skip-fast**: if the plist already points to the current exe AND the service is loaded AND the daemon socket answers, the call is a no-op. This eliminates the repeated "Background Items Added" Notification Center entries on every `ig update`.
- New `verify_daemon_health()` runs after every install / update: confirms the socket answers a real `projects_list` ping AND that exactly one `ig daemon foreground` process is running. Strays (test orphans from `target/{debug,release}/ig`, legacy `ig-rust` daemons surviving from the pre-v1.19 shim+backend layout) are SIGTERM'd. Uses `ps -axww -o pid=,command=` for portable cmdline matching (`pgrep -af` was Linux-only and silently returned bare PIDs on macOS).
- `post_update_rewarm` now actually reloads the daemon: previously it gated its restart on `is_daemon_available()`, so a crashed or never-installed daemon stayed dead across updates. New logic: if a service unit exists, call `install_launchd` (now idempotent) to reload via the service manager; else inline-restart; else print a one-liner pointing at `ig daemon install`.
- `install.sh` auto-runs `ig daemon install` at the end (opt-out via `IG_NO_DAEMON_INSTALL=1`), so a fresh `curl … | bash` no longer leaves the user with a binary but no daemon.

### Fixed — stable codesign identifier on macOS (stops TCC re-prompts)

Ad-hoc codesign with no `-i` embeds the binary hash into the Bundle ID (`ig-5555494468fc…`), so every rebuild looked like a brand-new app to the TCC database and BTM service. macOS would re-prompt for file-access permissions on every `ig update`.

- `install.sh` and `update.rs::atomic_install` now re-sign with `--identifier dev.makfly.ig` after every install. The Bundle ID stays constant across releases — TCC keys off the identifier when the team is unset, which lets you grant Full Disk Access (or accept the per-folder prompts) **once** and keep it forever.
- Note that the CDHash still changes per build; this is the expected price of ad-hoc signing. The identifier-stable approach gets you ~99% of the way to Developer-ID-style trust without the $99/year Apple Developer Program subscription.

## [1.19.11] — 2026-05-13

### Fixed — `session-start.sh` shebang broke on Linux

- `hooks/session-start.sh` shipped with `#!/opt/homebrew/bin/bash`, a macOS-Homebrew-ARM-only path. On Linux (and Mac Intel, where Homebrew lives at `/usr/local/bin/bash`), the kernel couldn't resolve the interpreter and the hook failed at every Claude Code session start with `/bin/sh: 1: …/session-start.sh: not found` — misleading, since the script itself existed and was executable.
- Switched to `#!/usr/bin/env bash`, matching the other hooks (`ig-guard.sh`, `subagent-context.sh`). Portable across macOS (system bash 3.2 or any Homebrew prefix) and Linux. Script body already uses only bash 3.2-compatible features (`[[`, `${var:-}`, `&>`, no assoc arrays).
- Run `ig setup` to refresh the installed hook in `~/.claude/hooks/`.

## [1.19.10] — 2026-05-12

### Added — `ig version` subcommand

`ig` parses bare words as a search shortcut (`ig "pattern"` = `ig search "pattern"`). Until now, typing the universal `ig version` silently searched for the word "version" across the project — a surprising no-op for new users coming from `cargo`, `gh`, `docker`, `kubectl`, etc., where `tool version` is canonical.

- New `Version` subcommand prints `ig <semver>` (same output as `ig --version` / `-V`).
- Excluded from `auto_gc` startup path — pure metadata query, no cache work.
- Search shortcut for *any other* word is unchanged (`ig foo`, `ig start`, `ig init` still search). Only `version` is intercepted, because it has a universal CLI convention and no legitimate code-search use case (developers searching for the literal token "version" can use `ig search version` or `ig -F version`).

## [1.19.9] — 2026-05-12

### Fixed — silent bash/fish shell hooks

- The managed shell hook (`# >>> ig managed >>>` block written by `ig setup`) used a bare `&` to background `ig warm` on every `cd`. On zsh this was paired with `&!` (disown), so nothing was printed. On **bash**, with no equivalent to `&!`, the job stayed under job control and the shell printed `[N] PID` at launch then `[N] Done` / `[N] Exit N` at every subsequent prompt — visibly polluting interactive sessions on Linux.
- **bash**: `cmd &` → `(cmd &)`. The sub-shell exits immediately so the parent never registers the job; no job-control notification can be emitted. POSIX-portable.
- **fish**: added explicit `disown` after each `&` to suppress fish's `Job N, '…' has ended` messages.
- Run `ig setup` (or any `ig update`, which refreshes managed blocks) to pick up the silent hook in `~/.bashrc` / `~/.config/fish/config.fish`. zsh users are unaffected.

## [1.19.8] — 2026-05-12

### Added — automatic cache GC

- `ig` now runs opportunistic cache GC on startup, at most once per hour by default.
- Auto-GC removes orphaned project caches, entries unused for 30 days, and least-recently-used entries when total cache size exceeds 5 GB.
- New manual cap: `ig gc --max-size 5GB [--dry-run]`.
- New config/env controls: `[cache] auto_gc`, `auto_gc_interval_secs`, `auto_gc_days`, `auto_gc_max_size_mb`; env `IG_AUTO_GC`, `IG_CACHE_GC_INTERVAL_SECS`, `IG_CACHE_GC_DAYS`, `IG_CACHE_MAX_SIZE_MB`.

### Fixed — 9 issues from external code review

Round of hardenings around daemon synchronisation, hook safety, and parity gaps. No on-disk format changes (`INDEX_VERSION` unchanged).

- **#1 `ig hold end` now blocks** until the worker has flushed buffered paths and bumped the seal. The IPC handler waits on a `SyncSender<()>` ack with a 30 s timeout; a follow-up search no longer hits a stale index.
- **#2 Daemon `--type` reuses the in-process alias resolver** (`ts→ts|tsx`, `rust→rs`, `python→py|pyi`, `c→c|h`, …). Daemon and non-daemon search now return identical file sets.
- **#3 `ig rewrite` is shell-injection-safe.** New POSIX single-quote helper escapes patterns/paths containing `$()`, backticks, `;`, `"`, `\`. The old `"{}"` double-quoting was vulnerable.
- **#4 Rewrites preserve command semantics.** `find some/subdir -name "*.rs"` keeps the search root, `git -C /tmp/repo status` is now passthrough (was silently dropped), `tree` no longer routes through the dead `.ig/tree.txt`.
- **#5 Daemon PID safety.** `stop_daemon` and `is_daemon_alive` verify the recorded PID is an `ig` daemon (Linux `/proc/<pid>/cmdline`, macOS `ps -p`) before SIGTERM. PID reuse after a crash can no longer signal an unrelated process.
- **#6 UTF-8 safe truncation.** Compact-output and analytics paths now use `str::floor_char_boundary` instead of raw byte slicing. Long lines containing emoji or accented characters no longer panic.
- **#7 Fresh projects appear in `by-name/` immediately.** `write_meta` now incrementally refreshes the symlink and the global manifest, instead of waiting for migration or GC.
- **#8 First-search auto-build for agents.** Non-TTY callers (Claude Code, codex, CI) now get a detached `ig index <root>` spawned on the brute-force fallback path, so the next call lands on a real index. Previously the auto-build was TTY-gated.
- **#9 Layout lock can no longer be stolen from a live holder.** `.layout.lock` now records the holder PID; another process only takes over when the PID is dead, or after `LAYOUT_LOCK_LIVE_TIMEOUT` (60 s) of polling.

### Added — E2E test harness

- `tests/e2e_review_fixes.rs` drives the real `ig` binary through subprocess invocations against an isolated `IG_CACHE_DIR`. Covers issues #1, #2, #3, #6 with no interference with the user's running daemon.
- Suite totals: **436 unit + 49 integration + 4 E2E tests passing.**

## [1.19.7] — 2026-05-07

### Added — daemon memory governor

`ig daemon` now has a Cursor-style RSS governor: it stays useful under normal load, sheds background state under soft pressure, and exits with a short cooldown under hard pressure so Claude/Codex hooks cannot relaunch it in a memory loop.

- Defaults: `daemon_soft_rss_mb = 768`, `daemon_hard_rss_mb = 1024`, `daemon_cooldown_secs = 60`.
- Soft pressure evicts tenant LRU caches and inactive watched projects, then pauses new background activations/rebuilds if RSS is still above the soft limit.
- Hard pressure removes the daemon socket/pid, writes `memory.cooldown.json`, and exits after a short grace period. Auto-start respects the cooldown.
- `ig daemon status` now prints current RSS plus configured soft/hard limits.
- New config/env controls: `IG_DAEMON_SOFT_RSS_MB`, `IG_DAEMON_HARD_RSS_MB`, `IG_DAEMON_COOLDOWN_SECS`, `IG_DAEMON_MAX_ACTIVE_PROJECTS`, `IG_INDEX_MEMORY_MB`, `IG_INDEX_BATCH_SIZE`.

### Changed — lower background indexing footprint

- Daemon active-project cap now defaults to 8 and idle project pruning to 5 minutes.
- Full-index SPIMI budget is configurable and defaults to 64 MB instead of the old hardcoded 128 MB.
- Full-index file batches are configurable and default to 250 files instead of the old hardcoded 1000.
- Semantic co-occurrence indexing remains enabled for explicit CLI indexing, but is disabled by default inside the daemon. Set `daemon_semantic_index = true` or `IG_SEMANTIC=1` if you want daemon warms to build it.

No on-disk format change, no `INDEX_VERSION` bump.

## [1.19.6] — 2026-05-07

### Fixed — multi-session agent holds

`ig hold begin/end` is now reference-counted per project. Multiple Claude Code or Codex sessions can hold the same project simultaneously; one `SessionEnd` no longer releases watcher rebuilds while another session is still active.

The watcher also checks the pre-flipped `session_active` atomic when processing file paths, closing the small race where filesystem events could arrive before the worker consumed the `SessionBegin` control message.

Tests: `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`.

## [1.19.5] — 2026-05-07

### Added — `ig hold` (agent edit-session lock)

v1.19.4 stopped the *idle* rebuild loop, but did not address the failure mode where an AI agent (Claude Code, Codex) legitimately edits 50–200 files in a few seconds: the watcher would still cross `OVERLAY_THRESHOLD = 100`, fall back to a full rebuild, and the next batch of events — fired during the rebuild — would do it again. RSS was observed at **19 GB / 96 % CPU** on `instant-grep` itself during a refactor session.

The fix borrows from Cursor's architecture: instead of trying to debounce smarter, treat the agent burst as an explicit transaction.

- New CLI: `ig hold begin|end|status [path]` (alias `session-hold`). Between `begin` and `end`, the daemon **buffers** dirty paths instead of rebuilding. On `end`, the buffer is sorted, deduped and hash-filtered, then folded into a **single** `update_index_for_paths` call.
- New IPC ops: `session_begin`, `session_end`, `session_status`.
- `ProjectStatus` gains `session_active: bool` and `session_pending: usize`, surfaced in `ig hold status` and `ig projects list --json`.
- `watch_worker` now consumes `WatchEvent::{Paths, SessionBegin, SessionEnd}` on a single `mpsc` channel, so session control events are ordered with respect to FS events naturally.
- `ig update` re-runs quiet setup so `~/.claude/` hooks/rules and `~/.codex/AGENTS.md` are updated to the current binary contract after every self-update.

Wiring into Claude Code (`~/.claude/settings.json` + `~/.claude/hooks/session-start.sh`):

```jsonc
"SessionStart": [{ "hooks": [
  { "type": "command", "command": "~/.claude/hooks/session-start.sh", "timeout": 5 }
]}],
"SessionEnd":   [{ "hooks": [
  { "type": "command", "command": "ig hold end   \"$CLAUDE_PROJECT_DIR\" 2>/dev/null || true" }
]}]
```

`session-start.sh` calls `ig hold begin "${CLAUDE_PROJECT_DIR:-$PWD}"` synchronously; it no longer launches a background `ig warm`, avoiding a rebuild window before the lock is active.

Codex CLI has no hook system; wrap manually with a shell function (see `CLAUDE.md`).

### Fixed — concurrent daemon starts

`ig warm`, `ig hold begin`, and search auto-spawn can all fire during agent startup. The daemon foreground path now takes a process-wide `daemon.lock` with `flock` and holds it for the server lifetime, so concurrent starts collapse to one global daemon instead of leaving orphan foreground processes bound to stale sockets.

### Fixed — release assets

The GitHub release workflow now uploads both the C shim (`ig-<platform>`) and Rust backend (`ig-<platform>-rust`) per platform, matching `install.sh` and `ig update`.

**Validated** with `claude -p` editing 25 files inside an active hold: daemon stayed at **85 MB RSS / 0 % CPU** during the entire burst, 175 FS events buffered, **one** overlay rebuild fired at `end` (50 unique paths after dedup). Without the hold, the same burst pushed the daemon to 16 GB / 99 % CPU.

**No on-disk format change, no `INDEX_VERSION` bump.** Old daemons returning `unknown op: session_begin` are auto-restarted via the existing `response_needs_newer_daemon` path.

Tests: 431 lib + 49 integration pass. `cargo clippy --all-targets -- -D warnings` clean. `cargo fmt` clean.

## [1.19.4] — 2026-05-07

### Fixed — daemon watcher rebuild loop (39 GB RSS / 310 % CPU)

A watched project on a busy monorepo (e.g. `next.js + symfony + bun` with hot-reload artefacts) could drive the global daemon into a self-amplifying rebuild loop, observed in the wild at **39 GB physical footprint, 310 % CPU sustained for 11 h**. Three independent bugs combined to produce it; v1.19.4 fixes all three in `src/daemon.rs`.

**Bug 1 — `is_ig_internal_path` skipped canonicalisation.**
`strip_prefix` failed silently when the watcher fired with `/var/folders/…` while the watched root resolved to `/private/var/folders/…` on macOS. The same pattern was fixed in `writer::normalize_changed_path` back in v1.17.1; the daemon copy was missed.

**Bug 2 — no content-hash check on watcher events.**
IDEs and dev-servers regularly `touch` files (mtime bump, identical bytes); each touch produced a real watcher event, an overlay rebuild, a seal bump and an LRU reload — for zero net change. `watch_worker` now keeps a per-project LRU of `(path → ahash)` (cap 4096, 5 MB max file size hashed) and drops events whose contents have not actually changed. Deleted / unreadable paths are still propagated so the writer can tombstone them.

**Bug 3 — watcher events were never pre-filtered by `.ignore`.**
The `notify` watcher recurses into the project root unfiltered, so `node_modules/`, `.next/`, `target/`, `vendor/`, `var/cache/`, `storage/logs/` and friends all surfaced events, blew past `OVERLAY_THRESHOLD = 100`, and triggered repeated full rebuilds (`Detected 1922 changed files (>100 threshold), full rebuild...` logged dozens of times in 90 s). The daemon now builds a `Gitignore` matcher at `ActiveProject::start` from `walk::DEFAULT_EXCLUDES` + `<root>/.ignore` + `<root>/.gitignore` and discards matching events before they hit the channel.

### Impact

On the reproducer (`headless-kit-next-php-hono` warmed in the daemon, no other activity):

| | v1.19.3 | v1.19.4 |
|---|---|---|
| RSS after 90 s idle | 822 MB and rising | 89 MB stable |
| CPU after 90 s idle | 81 % | 0 % |
| `daemon.log` lines / 90 s | 80+ rebuilds | 6 lines (one legitimate edit) |

### Tests

431 + 49 unit tests pass; `cargo clippy --all-targets -- -D warnings` clean; real test on macOS — daemon stopped, binary reinstalled (`codesign -fs -`), restarted, both `instant-grep` and `headless-kit-next-php-hono` warmed; daemon stayed at 0 % CPU and ~89 MB RSS over the test window.

### Also — `walk::DEFAULT_EXCLUDES` extended (Python coverage)

Five additional directory names are now excluded by default (so projects without an `.ignore` file are still safe): `.eggs`, `__pypackages__`, `.hypothesis`, `.ipynb_checkpoints`, `htmlcov`. The list already covered `__pycache__`, `.venv`, `venv`, `.mypy_cache`, `.ruff_cache`, `.pytest_cache`, `.tox`, `vendor`, `target`, `node_modules`, `.next`, `.nuxt`, `dist`, `build`, `.turbo`, `.output`, `.vercel`, etc.

### Notes for users

- The `.ignore` matcher is built once per project at warm time. If you edit `<root>/.ignore` while the daemon is running, restart it (`ig daemon stop && ig daemon start`) to pick up the new rules.
- The hash cache lives in memory only; nothing changed on-disk. **No `INDEX_VERSION` bump.**

---

## [1.19.3] — 2026-05-06

### Changed — drop version references from managed-block content

The Search Tools section that `ig setup` writes into `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, and `~/.claude/rules/tools/ig.md` no longer mentions specific version numbers ("since v1.15.0", "pre-v1.15.0 leftover"). Versions live in this CHANGELOG, not in steady-state agent rule files. Existing installs auto-detect the drift on next `ig setup` and rewrite the managed block.

### Documentation refresh

- `README.md` — TL;DR gets two new bullets covering the v1.19.0 cache layout (`daemon/`, `projects/`, `by-name/`, `tee/`, `manifest.json`) and the v1.19.1+ self-healing setup. New ASCII tree of the cache root. New subsection in Cache management documenting the migration path. File tree annotated with the v1.19 additions to `cache.rs` and `setup.rs`.
- `CLAUDE.md` (project) — version prefixes ("since v1.X") removed from the steady-state contract description; added an ASCII diagram of the cache layout and a note on `ig setup --quiet` behaviour.
- `docs/specs/SPEC-daemon-cache-invalidation.md` — already covers the seal contract introduced in v1.18.0; valid as-is for v1.19.x.

No code changes beyond the string constants. 431 lib tests passing.

---

## [1.19.2] — 2026-05-06

### Added — `ig setup --quiet` + auto-sync after `ig update`

`ig update` already called `setup::run_setup` post-self-update, but it printed the full noisy banner + per-agent skip lines for every untouched entry. With v1.19.1 introducing managed-block detection that runs on every setup, this became a wall of "already up-to-date" lines after every binary upgrade.

`--quiet` (also short `-q`) flips the output from "report everything" to "surface only what drifted":

| Output | Default | `--quiet` |
|---|---|---|
| Banner `🔧 ig setup …` | shown | hidden |
| `⊘ Windsurf — not detected` etc. | shown | hidden |
| `✓ Claude Code` agent header | always | only when an action ran |
| `→ Configured: …` lines | shown | shown |
| `→ already up-to-date` lines | shown (dim) | hidden |
| `✓ Shell hook` header + child line | always | only when changed |
| `Done! ig configured for N agent(s)` summary | shown | hidden |

Empirical: on a clean install where nothing has drifted, `ig setup --quiet` prints **zero bytes**. After modifying a managed section, it prints exactly the one line describing the fix (`→ Updated Search Tools section in ~/.claude/CLAUDE.md`).

### Changed — `ig update` auto-runs setup in quiet mode

`update.rs::post_update_rewarm` now calls `setup::run_setup_with_options(false, true)`. Effect: the user sees the same `Refreshing ig ecosystem…` line as before, but it only surfaces agent rule files that the new binary's contract actually changed. Most binary upgrades that don't touch the managed section will show nothing at all under "Refreshing".

### Implementation

- `src/cli.rs` — `Setup` subcommand gains `--quiet` / `-q` flag.
- `src/main.rs` — dispatches `Commands::Setup { dry_run, quiet }` to `run_setup_with_options`.
- `src/setup.rs`:
  - `run_setup_with_options(dry_run, quiet)` is the new entry point.
  - `QUIET_SETUP: AtomicBool` carries the flag into `print_results` without threading it through every `AgentSetup` impl. Reset to `false` at end-of-run so subsequent in-process calls aren't sticky.
  - `print_results` skips the agent header line when every action was idempotent and the flag is set.
  - The "not detected" / Shell hook scaffolding lines are gated on `!quiet || changed`.
  - The trailing `Done! ig configured for N agent(s).` summary is hidden in quiet mode.
- `src/update.rs` — switched to `run_setup_with_options(false, true)`.

431 lib tests passing, real test confirms zero output on clean state and exactly-one-line on drift.

---

## [1.19.1] — 2026-05-06

### Changed — `ig setup` rewrites stale `Search Tools` sections

Pre-v1.19.1, the setup flow detected an existing `## Search Tools` section in agent rule files and silently skipped — meaning users who installed ig before v1.19 (and lived through the cache layout move from `<root>/.ig/` to `~/.cache/ig/`) kept the **stale instructions referring to non-existent paths** until they manually wiped their CLAUDE.md.

This release introduces **managed-block sentinels** so the section is now find-and-replaced atomically across version bumps:

```markdown
<!-- IG-MANAGED-BLOCK:BEGIN -->
## Search Tools (`ig` — instant-grep)
... current contract: cache layout, daemon, commands ...
<!-- IG-MANAGED-BLOCK:END -->
```

Behaviour matrix per `ig setup` invocation against `~/.claude/CLAUDE.md`:

| Existing state | Action |
|---|---|
| File missing | Create with `# CLAUDE.md` header + managed block |
| Has managed-block markers + content matches | `AlreadyDone` (no write) |
| Has managed-block markers + content drifted | Replace between markers (preserves user content outside) |
| Has legacy `## Search Tools` heading (no markers) | Replace from heading to next `## ` (or EOF), wrap in managed block |
| No `## Search Tools` and no `# Global Rules` anchor | Append managed block at EOF |
| No `## Search Tools` but `# Global Rules` exists | Insert managed block above the anchor |

Same pattern for `~/.claude/agent-md` files (Codex CLI's `AGENTS.md`, Gemini's `GEMINI.md`, the `kilorules.md` for Kilo Code).

### Added — `~/.claude/rules/tools/ig.md` is now setup-managed

The deep-dive rule file referenced from `~/.claude/CLAUDE.md` is now created/overwritten by `ig setup`. Owned entirely by ig: re-running setup always brings the file back to the binary's current contract (commands, paths, anti-patterns). A trailer line declares this:

```markdown
*This file is auto-managed by `ig setup`. Manual edits are overwritten on the next run.*
```

### Implementation

- `src/setup.rs::IG_SEARCH_TOOLS_SECTION` — single source of truth, wrapped with `<!-- IG-MANAGED-BLOCK:BEGIN/END -->` sentinels.
- `src/setup.rs::IG_RULES_TOOLS_IG_MD` — content of the deep-dive rule file.
- `src/setup.rs::upsert_managed_block` — generic find-and-replace helper. Status discriminated as `Inserted`, `Updated`, `ReplacedLegacy`, `Unchanged`.
- `src/setup.rs::configure_claude_rules_ig_md` — direct overwrite (no markers needed; the file is fully owned by ig).
- 2 new tests: `test_claude_md_legacy_section_is_upgraded`, `test_claude_md_idempotent_after_managed_install`. 431 lib tests passing.

Real test on this machine: `ig setup` correctly upgraded a legacy `~/.claude/CLAUDE.md` (`Replaced legacy Search Tools section`), created the new `~/.claude/rules/tools/ig.md`, and a second invocation reported `already up-to-date`. No duplication, no stale content surviving.

---

## [1.19.0] — 2026-05-06

### Changed — XDG cache reorganized into a navigable layout

The `~/.cache/ig/` (or `~/Library/Caches/ig/` on macOS) tree was a flat root: hash dirs and `daemon.{sock,pid,log}` mixed in the same directory, opaque hash names with no human-friendly index, no log rotation. With ~140 cache entries on a busy workstation this got tedious to inspect.

New layout:

```
~/Library/Caches/ig/
├── daemon/                          ← daemon runtime state (sock, pid, log)
│   ├── daemon.sock
│   ├── daemon.pid
│   ├── daemon.log
│   └── daemon.log.1 [.2, .3, .4, .5]   ← rotated at 5 MB, last 5 kept
├── projects/                        ← per-project caches, hash-keyed
│   ├── 2e0c08507bb58341/
│   ├── 367dab9a3c2d060a/
│   └── ...
├── by-name/                         ← human-friendly symlinks
│   ├── distribution-app-v2 -> ../projects/2e0c08507bb58341
│   ├── instant-grep        -> ../projects/367dab9a3c2d060a
│   └── ...
├── tee/                             ← centralized tee output (was per-project)
└── manifest.json                    ← global registry: hash → root, name, size
```

Migration is automatic and idempotent. `ensure_layout()` runs at the entry of every command and:

1. Acquires `cache_root/.layout.lock` (create-only, lock-of-record).
2. **SIGTERM**s any pre-v1.19 daemon still running (PID file at the legacy path) so it stops recreating moved entries mid-migration.
3. Moves every `cache_root/<hash>/` into `cache_root/projects/<hash>/`. On hash dir collision (a stale daemon recreated the legacy entry), the **newer mtime wins** — the old `projects/` entry is dropped and the legacy is renamed in.
4. Moves `daemon.{sock,pid,log}` into `daemon/`.
5. Builds `by-name/` symlinks from each project's `cache-meta.json` (slug from project basename, suffixed with hash bits if collision).
6. Writes `manifest.json` (atomic via tmp+rename).
7. Drops the `cache_root/.layout-v1` marker — subsequent calls fast-path.

Concurrent ig invocations are safe: contenders wait up to 5 s for the marker; if the lock-holder dies, the next caller takes over. New file-system primitives:

```rust
pub fn cache_root() -> PathBuf;
pub fn daemon_dir() -> PathBuf;        // cache_root/daemon
pub fn projects_dir() -> PathBuf;      // cache_root/projects
pub fn by_name_dir() -> PathBuf;       // cache_root/by-name
pub fn tee_dir() -> PathBuf;           // cache_root/tee
pub fn manifest_path() -> PathBuf;     // cache_root/manifest.json
pub fn ensure_layout() -> Result<()>;
pub fn rebuild_symlinks() -> Result<()>;
pub fn rebuild_manifest() -> Result<()>;
pub fn rotate_daemon_log_if_needed();
```

The daemon's `socket_path()` / `pid_path()` / `log_path()` now resolve to `daemon/`. `start_daemon` calls `ensure_layout` and `rotate_daemon_log_if_needed` at boot.

### Backward compat

- Pre-v1.19 indexes are migrated automatically on first run of any command.
- Legacy daemons (v1.16-v1.18) listening on `cache_root/daemon.sock` are SIGTERMed during migration.
- `ig query`, `ig daemon status`, `ig warm`, etc. all keep their CLI surface; only the on-disk paths changed.

### Tests

- 430 lib tests (+5 since v1.18.0): `ensure_layout_creates_v19_dirs`, `ensure_layout_migrates_legacy_hash_dirs`, `ensure_layout_is_idempotent`, `rebuild_symlinks_creates_human_names`, `rebuild_manifest_writes_entries`.
- Real tests run on a live cache with 8 active projects: migration succeeded, daemon started cleanly on the new socket path, `ig -l function` on tilvest returned 1707 files (parity with `rg` confirmed).

### CLAUDE.md — testing policy

A new section in `CLAUDE.md` codifies the rule: **always run real tests in addition to unit tests** before declaring work done. Real tests catch what unit tests miss — daemon socket drift, on-disk layout corruption, codesign rejection on macOS, mmap survival across truncate. The v1.17.x stale-state bug shipped because real tests weren't run between code change and CI green; v1.19.0 catches similar failure modes in the cache-reorg by exercising the actual binary against the actual cache.

---

## [1.18.0] — 2026-05-06

### Added — `.ig/seal` atomic publish marker + FSEvents push

Replaces the v1.17.2 multi-file fingerprint with a single 16-byte seal file plus a `notify` watcher on `.ig/`. The daemon is now correct under the strongest contract we can express:

> **When the seal is observed at generation N, every artifact of generation N is guaranteed already published on disk.**

Architecture (full design captured in [`docs/specs/SPEC-daemon-cache-invalidation.md`](docs/specs/SPEC-daemon-cache-invalidation.md)):

```
.ig/seal  ←  16 bytes:  [u64 generation | u64 finalized_at_nanos]
                         atomic-renamed as the FINAL act of every rebuild.
```

- **`src/index/seal.rs`** (new, ~110 LoC, 4 unit tests) — `read_seal`, `bump_seal`, `current_generation`. Atomic publish via `tmp + fs::rename`. Malformed seal → `None`. Missing seal → generation `0` (back-compat with pre-v1.18 indexes).
- **`src/index/writer.rs`** — `bump_seal` called at the end of `build_index` (after the final `metadata.write_to`) and `incremental_overlay` (after `build_overlay`). The `Index is up to date` early-exit intentionally skips the bump — nothing changed, nothing to invalidate.
- **`src/daemon.rs`**:
  - `ReaderView.last_fingerprint` (× 7 stat tuples) → `cached_seal: Option<Seal>`. The full struct is compared so a wipe-and-rebuild that resets generation to 1 still triggers reload via `finalized_at_nanos`.
  - `reload_if_changed()` reads 16 bytes per query (≈ 1 µs cache-hot) and compares against the cached seal.
  - **NEW `ActiveProject._ig_watcher`** — a second `notify` watcher on the `.ig/` directory (`NonRecursive`). On `seal` / `seal.tmp` events it fires `reload_tenant_if_open`, propagating an out-of-band rebuild from another shell **without waiting for the next query**.

Push (FSEvents) + pull (per-query 16-byte read) form a hybrid scheme. **Push is best-effort** (`notify`'s reliability is uneven on macOS / NFS / SMB). **Pull is authoritative** — if push misses, the next query catches it.

### Tests

- 425 lib tests (+4 since v1.17.1).
- 2 new daemon regression tests: `test_seal_bumped_by_full_rebuild`, `test_reload_if_changed_observes_new_generation`.
- 4 new seal tests in `src/index/seal.rs`.

### Performance

Bench unchanged from v1.17.2 (the seal/push only affects the *reload* path — query hot-path is identical):

| Pattern (3 284 files) | matches | ig (ms) | rg 15.1.0 (ms) | gain |
|---|---:|---:|---:|---:|
| `createApp` | 2 | ~4.2 | 32.3 | **7.66×** |
| `Vue` | 45 | ~4.4 | 32.5 | **7.39×** |
| `axios` | 15 | ~4.5 | 31.4 | **6.98×** |
| `function` | 1 707 | 17.4 ± 0.8 | 37.4 ± 2.8 | **2.15×** |

Match parity verified on 8 patterns (zero divergence with rg).

---

## [1.17.2] — 2026-05-06

### Fixed — daemon stale state (atomic publish + multi-file fingerprint)

The global daemon could serve `total_files=base_count` (overlay invisible) until restart. Three independent vectors hardened:

- **`src/index/reader.rs`** — `OverlayReader::open(...).unwrap_or(None)` silently swallowed parse/IO errors. Replaced with explicit `match { Ok(opt) => opt, Err(e) => { eprintln!(...); None } }` so the daemon log surfaces the failure instead of degrading to base-only in silence.
- **`src/index/{merge,metadata}.rs`** — `lexicon.bin` / `postings.bin` / `metadata.bin` were written via in-place `File::create` (O_TRUNC). On macOS, the kernel keeps the pre-truncate inode alive for any pre-existing `mmap`, masking the rebuild. Switched to `tmp + fs::rename` atomic publish for all three (overlay artifacts were already atomic).
- **`src/daemon.rs`** — replaced single-mtime `metadata_mtime()` with an `IndexFingerprint` capturing `(mtime, size)` for every artifact (metadata, lexicon, postings + 4 overlay files). Defends against sub-second `mtime` granularity collapse on rapid rebuilds: `size` always differs across a real rebuild even if `mtime` ties.

This fingerprint scheme was itself superseded by v1.18.0's `seal` (single 16-byte file, single read per query). The atomic-rename and silent-swallow fixes ship from this release forward.

### Tests

- 3 regression tests:
  - `corrupt_overlay_meta_returns_err_not_ok_none` — locks the no-silent-swallow contract.
  - `missing_overlay_meta_returns_ok_none` — sanity for the legitimate empty-overlay case.
  - Two daemon fingerprint tests (later replaced by seal tests in v1.18.0).
- 421 lib tests passing.

---

## [1.17.1] — 2026-05-06

### Added — precision search (vbyte codec, masked n-grams)

Squash-merge of `test/iso-cursor-precision-speed` into the v1.17.0 watcher daemon. Conflicts in `daemon.rs` / `writer.rs` / `update.rs` resolved by union (kept `QueryResponse` shape from main + the precision pipeline from the branch).

- **`src/index/vbyte.rs`** (+508 LoC) — varbyte posting list codec with `PostingEntry` masks. Sub-byte filtering before any `read(2)`.
- **`src/index/reader.rs`** (+694 LoC) — query path rewritten on three masks per posting entry:
  - `bloom_mask: u8` — set bit indexed by hash of the byte that follows the n-gram occurrence (3.5-gram filter).
  - `loc_mask: u8` — set bit per occurrence position (adjacency filter).
  - `zone_mask: u32` — exact small-position mask with an "overflow" bit for later bytes.
- **`src/query/extract.rs`** (+185 LoC) — `regex_to_query_costed` accepts a cost-estimation closure (the daemon plugs in its own `IndexReader::estimate_query_cost`).
- **`src/index/{merge,ngram,overlay,spimi,postings}.rs`** — posting masks plumbed through every layer.

`INDEX_VERSION` 10 → **13** — forces an automatic rebuild on first run after upgrade. Pre-v1.17.1 indexes are detected, rebuilt transparently, no user action required.

### Fixed — macOS canonicalize race in the watcher

`writer::normalize_changed_path` now canonicalizes the absolute path before `strip_prefix`. Without this, on macOS the watcher silently dropped every event because `tempfile::tempdir()` returns `/var/folders/…` while `root.canonicalize()` resolves to `/private/var/folders/…` — `strip_prefix` always failed and the rebuild never fired.

### CI follow-up

Commit `4879664` (`fix(v1.17.1): unbreak CI — clippy + rustfmt`) lands fast-follow corrections for 8 Clippy errors and 1 rustfmt diff that slipped past local checks during the squash-merge:
- Type aliases `NgramMaskEntry`, `ChangedFileEntry` for `clippy::type_complexity`.
- `collapsible_if` → 3 sites collapsed (with `&& let` chains where applicable).
- `manual_is_multiple_of`, two `needless_borrow` cases.
- `daemon.rs` import order normalized.

---

## [1.17.0] — 2026-05-05

### Added — `ig warm` and `ig projects {list,forget}` (daemon-watched projects)

The daemon now keeps a managed set of *active* projects. Each warmed project gets its own `notify` watcher, an LRU-residency guarantee, and an auto-rebuild loop in the background.

```bash
ig warm                  # warm the current project (idempotent)
ig projects list         # show every active project + idle seconds
ig projects forget <root>  # drop a project from the active set (frees its watcher)
```

Shell hooks (zsh / bash / fish) and the session-start hook now use `ig warm` instead of `ig daemon start`. The managed shell-hook block is **self-updating across versions** — re-running `ig setup` rewrites it in place.

### Added — `grep` rewrite preserves `-F` and `-l`

The pre-tool-use rewrite engine (`src/rewrite.rs`) now keeps `-F` (fixed-strings) and `-l` (files-with-matches) flags when transforming `grep` invocations into `ig`. Agents that emit `grep -Fl 'literal' src/` get the right behavior without surprises.

---

## [1.16.3] — 2026-04-29

### Fixed — `.ig-entries-*.tmp` orphan sweep on build start

`tempfile::NamedTempFile::persist()` detaches RAII cleanup, so any `SIGKILL` / OOM / panic during `merge::merge_segments_streaming` left the entries temp file behind. On busy projects these piled up fast — observed 5 919 orphans / 12 GB in `distribution-app-v2` — and every subsequent build slowed because `tempfile_in()` had to scan a saturated directory for a free name.

Fix: `sweep_orphan_entries(index_dir)` — non-recursive, exact match on `.ig-entries-` prefix + `.tmp` suffix, ignores anything else, logs under `IG_DEBUG` only. Called at the top of `merge_segments_streaming` and again at the start of `full_rebuild` for opportunistic early cleanup. Per-file errors (concurrent removal) are tolerated.

Verified on `distribution-app-v2`: **5 919 → 0 orphans, 12 GB → 92 MB**.

---

## [1.16.2] — 2026-04-29

### Fixed — CI green again

Compagnion fix to v1.16.0/v1.16.1: rustfmt diffs cleaned up, dead code paths from the per-project daemon era dropped. No behavior change.

---

## [1.16.1] — 2026-04-29

### Fixed — `daemon.pid` written in foreground mode

Before this fix, the global daemon only wrote its PID file when started via `ig daemon start` (which forks `daemon foreground` as a child). When launched directly by `systemd-user` or `launchd`, the PID file was missing and `ig daemon status` reported "not running" even though the daemon was happily serving queries on the socket.

The `ctrlc` handler now removes both the socket *and* the PID file on `SIGINT` / `SIGTERM`.

---

## [1.16.0] — 2026-04-29

### Changed — single global daemon (multi-tenant) replaces per-project mode

The old design ran one `ig daemon` per project. With the XDG cache from v1.15.0 making indexes addressable by hash, that model became pure overhead: each daemon paid the full Rust runtime + LRU caches + `notify` watcher cost. On a workstation with 16 cached projects this added up to **~995 MB of RSS** for state that's almost entirely cold.

This release collapses everything into a single daemon serving every project on the machine.

#### Architecture

- **One Unix socket**: `~/.cache/ig/daemon.sock` (XDG-aligned).
- **One PID file**:    `~/.cache/ig/daemon.pid`.
- **One log**:         `~/.cache/ig/daemon.log`.
- **`GlobalState`** holds an `LRU<root, Arc<TenantState>>`, default cap **32** (configurable via `IG_DAEMON_TENANTS_MAX`).
- Each tenant **lazily opens its `IndexReader`** on first query and keeps its own per-tenant regex / `NgramQuery` LRU caches.
- `mtime` polling on `metadata.bin` replaces the per-tenant `notify` watcher (cheaper, fewer threads). v1.18.0 supersedes this with the `seal` file.

#### Wire protocol

`QueryRequest` gains a `root` field. Clients (`ig query`, the auto-route in `do_search`) canonicalize the project root and send it inside the JSON payload — no more hash-of-path socket names per project.

#### Migration

- On startup, the new daemon scans `/tmp/ig-*.sock`, `SIGTERM`s each socket's owner via `lsof`, and removes the file. Idempotent.
- `ig daemon start|stop|status|install|uninstall` keep their CLI surface; the path argument is accepted for backward compat with old `launchd` / `systemd` invocations but ignored at runtime.
- `install_launchd` now writes a single `com.ig.daemon.global` plist (macOS) or a single `systemd-user` unit `ig-daemon.service` (Linux).

#### Performance

RAM measured on a 3-tenant smoke: **7.3 MB total** (vs ~180 MB before this release for the same 3 projects). Workstation wins are ~14×.

---

## [1.15.0] — 2026-04-29

### Changed — XDG cache for indexes (default location)

Indexes now live under `~/.cache/ig/<hash-of-root>/` by default instead of `<root>/.ig/`. Two motivations:

1. Non-versioned projects (Next.js scratchpads without `.git/`) were accumulating stray `.ig/` in every subdirectory the agent searched from.
2. The single-global daemon (v1.16.0) needs hash-addressable indexes anyway.

Backwards-compatible: an existing `<root>/.ig/` keeps being used for that project; set `IG_LOCAL_INDEX=1` to force local mode for new projects.

### Changed — `find_root` recognises project markers

In addition to `.git/`, the root walker now recognises `package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`, `deno.json`, `composer.json`, `bun.lock`, … and walks to the **highest** match. A search from `apps/web/` inside a monorepo now resolves to the monorepo root, not to a stray `node_modules/<pkg>/.git/`.

### Added — cache management subcommands

```bash
ig gc [--days N] [--dry-run]   # prune orphan / stale cache entries
ig migrate [--dry-run]          # move <root>/.ig/ to the XDG cache
ig cache-ls                     # list cache entries with size + last_used
```

Each cache entry carries `cache-meta.json` (`root_path`, `created_at`, `last_used_at`, `ig_version`) so `gc` can tell what's safe to drop.

---

## [1.14.2] — 2026-04-27

### Added — `ig emb on/off/status` (runtime embedding toggle)

Two layers of control now gate the OpenAI embedding playground:

| Layer | Mechanism | What it controls |
|---|---|---|
| **Compile-time** | `cargo build --features embed-poc` | Whether the `embed-poc` subcommand is present in the binary at all (default: absent). |
| **Runtime** | `ig emb on/off` (this release) | Whether the subcommand actually executes when present (default: off). |

Both are independent. The runtime toggle persists in `~/.config/ig/embed.toml`:

```toml
# Runtime toggle for `ig emb` — overridable with `ig emb on/off`.
enabled = false
```

#### Usage

```bash
ig emb status   # inspect current state (default: disabled)
ig emb on       # accepts: on, true, 1, yes, y, enable, enabled
ig emb off      # accepts: off, false, 0, no, n, disable, disabled
```

When `embed-poc` is **off** at runtime and the user calls `ig embed-poc <op>`:

```
Error: embeddings are disabled.
Enable with:  ig emb on
(or build a binary without the embed-poc feature to remove the subcommand entirely.)
```

When the cargo feature is OFF, the toggle still works (the file is written) but the subcommand simply doesn't exist — `ig emb status` prints a `note:` directing users to rebuild with `--features embed-poc`.

#### Why two layers?

- Compile-time off (default): published binary has zero OpenAI client code, no `tiny_http`, no API-key prompts. Dependency-clean for distribution.
- Compile-time on + runtime off (default in dev builds): user can experiment locally without accidental network calls. `ig emb on` is an explicit, auditable action.

#### Implementation

- `src/embed_toggle.rs` (~90 LoC) — read/write helpers + 3 unit tests (default-disabled, round-trip, malformed-config-falls-back-to-disabled).
- `cli.rs` adds a top-level `Emb { state: Option<String> }` always-available subcommand.
- `main.rs` gates `Some(Commands::EmbedPoc { op })` on `embed_toggle::is_enabled()` before dispatching to any phase.
- Fail-closed: if the config file is unreadable or malformed, embeddings stay off.

---

## [1.14.1] — 2026-04-27

### Fixed — refuse to auto-index `$HOME` / `/` / system roots

Reported in the wild: a user on Debian (still on v1.6.0) ran `ig update` from his home directory. v1.6.0 had no `Update` subcommand yet, so clap's positional fallback parsed `update` as a search pattern with default path `$HOME`. The auto-index walk then tripped on `~/Docker/mariadb/data/performance_schema` (mode `0700`, owned by user `mysql`) → `Permission denied` → `Error: walking files`.

v1.14.0 already silently skipped `Permission denied` during iteration (`Err(_) => continue` in `walk_files`), so the exact crash is gone — but searching from `$HOME` would still pointlessly brute-force-walk tens of GB of Docker volumes, mail spools, `~/Library`, `~/.cache`, etc. before producing anything useful.

**Fix:** new `guard_suspicious_root` in `src/main.rs` refuses to auto-build an index OR fall through to brute-force search when the resolved root is `$HOME`, `/`, `/usr`, `/home`, `/Users`, `/var`, or `/tmp` AND no `.ig/` already exists. Errors out with an actionable hint:

```
Error: refusing to auto-build an index for /home/julien — this is not a project root.

Walking it would crawl system directories (Docker volumes, mail spools, protected
subtrees) and is almost never what you want.

Hint: `cd` into a real project first, or pass an explicit path:
  ig "<pattern>" /path/to/your/project

To override (you really meant it), re-run with IG_ALLOW_HOME_INDEX=1.
```

The escape hatch `IG_ALLOW_HOME_INDEX=1` lets users who deliberately index their `$HOME` (e.g. running a personal-knowledge-base index) opt back in. The guard never fires when an existing `.ig/` is found at the root, so previously-built `$HOME` indexes keep working.

Two call sites are gated:
- `ensure_index` (explicit `ig daemon start/install`, `Symbols`, etc.)
- `do_search` (the implicit search path that powers `ig "pattern"`), before the brute-force fallback can touch the filesystem

v1.6.0 and earlier users still need to upgrade since their binary doesn't ship the guard.

---

## [1.14.0] — 2026-04-27

### Changed — token compression beats `rtk` on most commands

Benchmarked against [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk) v0.35 on a real Turbo monorepo. `ig` wins on **14 of 16** commands; remaining two losses are within 2 % of `rtk`.

| Command | raw | rtk | ig | Δ vs rtk |
|---|---:|---:|---:|---:|
| `git log -10` | 5 496 | 2 779 | **1 109** | **−60 %** |
| `git log -50 --stat` | 139 470 | 13 901 | **7 938** | **−43 %** |
| `git log -20 -p` | 1 413 219 | 4 865 | **3 095** | **−36 %** |
| `git diff HEAD~5` | 303 082 | 26 578 | **11 312** | **−57 %** |
| `git diff HEAD~20` | 1 459 755 | 38 961 | **23 947** | **−39 %** |
| `git show HEAD` | 63 375 | 23 856 | **5 941** | **−75 %** |
| `git status` | 582 | 201 | **153** | **−24 %** |
| `find -name '*.ts'` | 23 400 | 1 270 | **105** | **−92 %** |
| `grep -rn 'async function'` | 17 816 | 113 | **22** | **−81 %** |
| `wc <large file>` | 68 | 18 | **17** | **−6 %** |
| `env` | 4 098 | 1 997 | **739** | **−63 %** |
| `cat <22 KB .ts>` (auto -s) | 22 694 | 22 694 | **983** | **−96 %** |

**`src/git.rs`** — `git log` collapses verbose flags (`--stat`, `--numstat`, `--name-*`, `-p`, `--patch`, `--raw`) into a single `--shortstat` per commit; `--oneline` uses tightest `%h %s` format; per-line cap at 120 chars; global cap at 16 KB with truncation marker.

**`src/ls.rs`** — drop the `X files, Y dirs` footer when entries ≤ 8.

**`src/cmds/run.rs`** — `route_to_dedicated` strips env-var prefixes (`IG_COMPACT=1 ig …`) and accepts the positional-pattern search shortcut, unblocking `grep`/`find` rewrites.

**`filters/system.toml`** — new compact `wc` (drops path, unit-suffixed counts) and `env` (drops shell internals + tooling caches, masks secrets, truncates at 200 chars). Replace patterns now use the `${1}L` regex-backref form.

### Added — `embed-poc` cargo feature (OFF by default)

The Phase-1/2/3 OpenAI embeddings POC is gated behind a cargo feature flag. The published `ig` binary ships **without** any OpenAI client code, no `tiny_http` server, no API-key prompts. Build with:

```bash
cargo build --release --features embed-poc
```

to enable `ig embed-poc {hello,index,inspect,search,serve}`. Fallback for users without an OpenAI key is the regular trigram path: `ig search "pattern"` — sub-millisecond, no network, no cost.

### Security

`.env` remains gitignored, pre-commit hook blocks `sk-[A-Za-z0-9]{20,}` and `OPENAI_API_KEY=<non-placeholder>`. Verified clean before commit.

---

## [1.13.0] — 2026-04-27

### Added — pure sparse n-grams (Phase 1, INDEX_VERSION 11)

Removed the legacy fixed trigram fallback path. Index relies entirely on the danlark1/sparse_ngrams covering algorithm — fewer, longer keys → smaller posting lists, smaller candidate sets, smaller `.ig/`. Lexicon and postings shrink ~25–35 % on the iautos monorepo (3049 files: 31 MB → 22 MB lexicon, 7.1 MB → 5.0 MB postings). `INDEX_VERSION` bumped to **11**; existing v10 indexes are auto-rebuilt on the first query (no user action required).

### Added — C shim + hidden Rust backend (dual-binary install)

`ig` is now distributed as two artefacts:

- **`~/.local/bin/ig`** — a 35 KB C shim in the user `PATH`. Hot path (`search`, `grep`, `files`, `count`) parses argv, resolves the project root, opens the daemon socket, and prints results without ever leaving C (cold start < 2 ms vs ~12 ms for a full Rust boot). Cold path (`index`, `setup`, `update`, …) `execve`s the backend.
- **`~/.local/share/ig/bin/ig-rust`** — the 5.1 MB Rust backend, *outside* the `PATH`. Resolved by the shim through a 4-step fallback: `$IG_BACKEND` → user share → system share (`/usr/local/share/ig/bin/`) → first `ig-rust` on `PATH`.

Net effect: a single `ig` name in the user's `PATH` (no leaked `ig-rust` shadowing other tools), faster hot queries, and a clean uninstall surface. New tests in `shim/test_fallback_paths.c` (5/5) cover every fallback branch.

### Added — native `.ignore` autoignore (`src/autoignore.rs`)

`ig` now writes a `.ignore` at project root on first run, mirroring the 38 default-excluded directories (`node_modules`, `target`, `vendor`, `.git`, …). Lets `rg` and friends respect the same exclusions, and lets users edit it without touching `ig` config. Idempotent (skipped if the file already exists).

### Changed — `install.sh` rewrite for dual-binary layout

- Detects and migrates legacy single-binary installs (`~/.cargo/bin/ig`, `/usr/local/bin/ig-rust` are removed, `~/.local/bin/ig` is replaced by the shim).
- Downloads `ig-shim-<platform>` → `~/.local/bin/ig`, downloads `ig-backend-<platform>` → `~/.local/share/ig/bin/ig-rust`.
- Atomic file writes (temp + rename) for both artefacts.
- Idempotent: re-running upgrades both binaries in place without leaving a half-installed state on hook/SIGINT.

### Changed — `ig update` and `ig uninstall` are dual-binary aware

- `ig update`: `resolve_install_targets()` discovers both the running shim path *and* the backend path (env var, share dirs, `PATH` lookup). Downloads both artefacts, falls back to a single-binary release tarball if the v1.13.0 split assets 404 (forward-compatible with older self-hosted mirrors). Writes both atomically.
- `ig uninstall`: removes shim + backend + the `~/.local/share/ig/bin/` parent dir if empty. 4 new tests cover the hidden-backend branch.

### Tests

- `cargo test --lib` — 442 passing (was 438), including 4 new uninstall tests.
- `make -C shim test` — 13/13 (8 fallback + 5 fallback_paths).
- `which ig-rust` → not found (correct: backend hors `PATH`).

### Migration notes

Users on 1.11.x who run `ig update`:

1. Download split artefacts (shim + backend).
2. Atomically replace `~/.local/bin/ig` with the shim.
3. Install the backend at `~/.local/share/ig/bin/ig-rust`.
4. Remove any legacy `ig-rust` from `~/.cargo/bin/` or `/usr/local/bin/` to avoid PATH shadowing.

Existing `.ig/` indexes (v10) are auto-rebuilt on the next query — no manual `ig index`.

## [1.11.0] — 2026-04-25

### Added — auto-route CLI through daemon (transparent)

Each `ig "<term>" path` invocation used to re-pay binary cold start, re-mmap the index, and prefault the page-cache from cold every single time. The daemon (sub-millisecond hot queries) existed but was opt-in via the explicit `ig query` subcommand — so Claude Code, Codex and similar tools never benefited from it.

`do_search` now silently attempts a daemon round-trip first, falls back transparently to in-process `search_indexed` when the daemon is missing or the request is not representable, and on a fall-back spawns the daemon in the background (silent variant — no stderr noise) so the *next* call lands on a hot daemon. The route only fires for daemon-representable requests: no `--json`, `--stats`, `--top`, `--glob`, `--semantic`, no asymmetric context, no path filters.

Two opt-out env vars: `IG_NO_DAEMON=1` (skip the route entirely) and `IG_NO_AUTO_DAEMON=1` (skip the implicit spawn).

New public API in `daemon`: `DaemonResponse` + `DaemonMatch` typed structs (replacing the ad-hoc `String` return of `query_daemon`), `is_daemon_available(&Path) -> bool` (TOCTOU-safe liveness check — PID alive *and* socket bound), `try_query_daemon(...) -> Result<Option<DaemonResponse>>` (`Ok(None)` when unreachable so callers fall through), and `start_daemon_background_silent`.

### Added — `IndexReader::warm_lexicon()`

Symmetric to the existing `warm_postings`. The lexicon mmap was previously hinted with `MADV_WILLNEED` at `IndexReader::open` but the kernel may delay the prefetch — on an 80+ MB lexicon the first few queries would otherwise eat random page faults during linear probing. The daemon now calls `warm_lexicon()` during its boot warm-up phase, so no query ever sees a cold lexicon.

### Fixed — empty `path_filter` when path equals project root

`resolve_root_and_filters(["."])` produced `path_filter = "/"` whenever the provided path was already the project root: `rel_str` came out empty, the trailing-slash normalisation pushed a lone `/`, and downstream `search_indexed` filtered against `rel_path.starts_with("/")` — which never matches because stored rel paths never have a leading slash.

Net effect: every `ig "<term>" .` invocation returned `0 matches` silently even though the index was correct. The daemon path was unaffected because it ignores `path_filters` entirely; the bug therefore only surfaced on the in-process indexed path and was masked any time the daemon answered.

Fix: when the resolved relative path is empty, skip pushing a filter at all instead of normalising it to `/`. Predates the auto-route work but shipped together because the auto-route bench surfaced it.

### Performance

Four small, additive optimisations on the verify path and indexation hot path.

- **memchr SIMD newlines** in `matcher::match_file`. `line_starts` was built with a byte-by-byte scan; replaced by `memchr::memchr_iter` (SSE2/AVX2 on x86, NEON on aarch64) — 3-10× faster on large files. Adds a `Vec::with_capacity(content.len() / 40 + 1)` hint so realloc churn drops to ~zero on source code.
- **Per-worker regex clone via rayon `map_init`** in `search::indexed::search_indexed` and `daemon::process_query_cached`. The candidate-verification `par_iter` used to clone the compiled regex once per file (to dodge regex#934 internal-pool contention). `map_init(|| regex.clone(), |re, item| ...)` clones once *per worker thread* instead — ~16× fewer clones at the default rayon pool size.
- **`vbyte::decode_u32` / `encode_u32` → `#[inline(always)]`**. Inner loop of every posting-list decode (millions of calls per query); the plain `#[inline]` hint was respected only sometimes by rustc.
- **`bigram_df` set bucket cap** in `writer.rs`. The per-file `AHashSet<u32>` for unique-bigram collection was pre-allocated with `bytes.len()` capacity — so a 100 KB source file reserved ~1.5 MB while in practice holding ~8 K bigrams. Capped at 8192 and shipped directly (no intermediate `Vec<u32>`); sizable drop in indexation peak RAM on large repos.

Adds `memchr = "2"` to `[dependencies]`; resolver picks the same crate ripgrep already pulls in transitively, so the dep-tree weight is flat.

### Benchmarks — iautos/apps (3049 files, 100 MB index, warm cache)

`hyperfine --warmup 3 --runs 12 -N`:

| pattern             | v1.10.1 (no daemon) | v1.11.0 (daemon) | gain  |
| ------------------- | ------------------: | ---------------: | ----: |
| `useEffect`         | 7.2 ms              | **5.7 ms**       | -21 % |
| `createServer`      | 3.8 ms              | **2.6 ms**       | -32 % |
| `fn\s+\w+_test`     | 4.1 ms              | **3.0 ms**       | -27 % |
| `async function`    | n/a                 | **8.1 ms**       |       |

Burst of 10 sequential queries (representative of an agent's pattern):

| metric                      | v1.10.1   | v1.11.0   |
| --------------------------- | --------: | --------: |
| Total wall time (5 runs)    | 84.6 ms   | **72.3 ms** (-15 %) |
| User CPU time               | 61.7 ms   | **18.8 ms** (-70 %) |

### Benchmarks — ig vs ripgrep 14.1.1 (same workload)

Match counts identical on all 5 patterns (zero divergence — file count *and* total match count match `rg` byte-for-byte).

| pattern             | ig (daemon) | rg 14.1.1 | ig faster |
| ------------------- | ----------: | --------: | --------: |
| `useEffect`         | 5.9 ms      | 18.3 ms   | **3.1×**  |
| `createServer`      | 2.4 ms      | 18.8 ms   | **7.8×**  |
| `fn\s+\w+_test`     | 3.5 ms      | 27.4 ms   | **7.8×**  |
| `async function`    | 8.1 ms      | 18.2 ms   | **2.2×**  |
| `export default`    | 6.9 ms      | 18.0 ms   | **2.6×**  |

`rg` spends ~17-27 ms walking the gitignore tree and opening the ~3000 candidate files; `ig`'s trigram filter cuts that to ~50-200 candidates *before* any file is touched — `User: 1.5 ms, System: 1.5 ms` average.

### Tests

`cargo test --release --no-fail-fast` — **438 passing**, 0 failures.

## [1.10.1] — 2026-04-24

### Changed — `ig gain` dashboard surfaces usage-only commands

The savings table sorts by `saved_bytes` desc, so high-volume commands with no honest byte baseline (typically `ig search`, `ig read` without flags, `ig smart`, …) were pushed off the top-20 view. They've always been logged via `tracking::log_usage`, just invisible.

New *"By Usage (no byte baseline)"* section below the main table: top-10 commands by count with `saved_bytes == 0`. No fabricated multipliers — `ig search` output is byte-identical to `grep -rn`, so claiming savings there would be dishonest. The section just shows volume.

Example: a workflow with 1 k `ig search` calls now lists them explicitly instead of hiding them under a "151 total commands" footer.

## [1.10.0] — 2026-04-24

### Added — BM25 ranking with `--top N`

New `--top N` global flag on `ig search`. When set, the matched files are scored with a textbook Okapi BM25 and only the top-N are returned. `tf` is the per-file match count, `df` is derived from the result set, `dl` is the file byte-size, `avdl` is the mean across matches. `k1 = 1.5`, `b = 0.75`.

```bash
ig --top 5 useState
# returns the 5 files with the richest useState usage (dense hits in short files first)
```

Because the scoring happens after the trigram pre-filter, the overhead is only a `stat(2)` per candidate — no second regex pass. New module `src/search/rank.rs` (3 tests).

### Added — `--semantic` PMI query expansion (no ML model)

New global flag: `ig --semantic <word>` expands a single-word literal query to `\b(word|n1|n2|…|n6)\b` using the top six co-occurring tokens learned from the corpus during indexing. The synonyms are chosen by count-weighted **Pointwise Mutual Information** (`pmi · log(count + 1)`), which Levy & Goldberg (2014) proved is the objective skip-gram word2vec implicitly optimises — so we recover most of a learned embedding's neighbourhood quality with zero ML runtime, zero model download, zero GPU.

```bash
ig --semantic throw
# (semantic: expanded 'throw' → got, inattendu, denied, autorisé, trouvée, manquant)
# …matches throws, error handlers, and French exception messages in one pass
```

- Co-occurrence table lives at `.ig/cooccurrence.bin` (bincode, ~1.5 MB on a 3 k-file repo).
- Built automatically as a second pass during `ig index`. Disable with `IG_SEMANTIC=0 ig index`.
- Tokenizer splits `camelCase`, `snake_case`, `kebab-case`, acronyms (`HTTPRequest` → `http`, `request`), drops 40 stop-words + JSON `\uXXXX` escape artefacts + pure numbers + tokens shorter than 2 chars.
- 16 new tests (`src/semantic/tokenize.rs` + `src/semantic/cooccur.rs`).

New modules: `src/semantic/{mod,tokenize,cooccur}.rs`.

### Added — auto-compact on pipe + path ellision

`Printer::compact_limits()` now activates compact mode automatically when `!stdout.is_terminal()` (unless `IG_COMPACT=0` forces verbose). In that mode:

- Long paths in per-file headers are ellided: `apps/pwa-backoffice/src/app/.../maintenance-client.tsx` → `apps/.../components/maintenance-client.tsx`.
- Line width capped at 80 (aligned with rtk's default).
- Empty result now emits a single `0 matches for "pattern"` so an agent distinguishes "no hit" from "tool crashed".

### Added — `ig files` and `ig smart <dir>` aggregate mode

Both commands now emit a one-block aggregate instead of enumerating every item when stdout is a pipe and the input is a big tree:

```text
$ ig files
3201 files in 911 dirs · 972 tsx, 890 php, 790 ts, 80 mdx, 70 py, 47 json
(compact view — set IG_COMPACT=0 or run in a TTY for the full listing)

$ ig smart apps/api
apps/api: 1042 files, 249 dirs · 890 php, 39 yaml, 31 twig, 29 sh, 10 md, 7 ini
top: src/ (664), migrations/ (109), tests/ (103), config/ (42), @docker/ (39)
key: composer.json, README.md, Makefile
```

On the iautos monorepo: `ig files` drops from 176 KB to 149 B (≈1 180×), `ig smart apps/api` drops from 69 KB / 5.3 s to 345 B / 19 ms (≈200× smaller, ≈280× faster).

### Changed

- `ig gain` default table shows **top 20** instead of top 15. Use `ig gain --full` for the full list.

### Benchmark — ig beats rtk on aggregate (first time)

115 cases on a 347 k-file monorepo (`iautos`) against `rtk 0.37.2`. Methodology: 2 warm-up passes + median of 3 wall-time runs per case.

| | ig | rtk |
|---|---:|---:|
| Total bytes emitted | **896 KB** | 1.04 MB |
| Total wall time | **1.74 s** | 2.88 s |
| Bytes wins | **57 / 115** | 54 / 115 *(tie: 4)* |
| Time wins | **80 / 115** | 27 / 115 *(tie: 8)* |

Categorically-ahead domains (rtk has no persistent index, so these remain structural wins): `--top N` BM25 = **10/10 bytes wins**, `--semantic` = **5/5 bytes wins**.

Full per-domain table + raw CSV in `documentation/public/bench/v1.10.0/`.

## [1.9.2] — 2026-04-23

### Fixed — `ig setup` / `ig update` now actually propagate hook changes

Prior to 1.9.2, `ig setup` was fully idempotent but **non-upgrading**: once a hook file or a settings.json entry existed, it was never touched again, even when a newer binary shipped a fixed version of the same hook. In practice this meant users running `ig update` from 1.9.0 → 1.9.1 kept the broken `$CLAUDE_BASH_COMMAND`-only hook on disk.

Two call sites were fixed in `src/setup.rs`:

- **`install_hook_file`** (hook `.sh` files in `~/.claude/hooks/`): now compares shipped content against what's on disk. Identical → `AlreadyDone`. Different → rename existing to `<name>.bak-<unix-ts>` and write the new one. Missing → install fresh. Message reports `Installed` vs `Updated` explicitly.
- **`ensure_hook_registered`** (inline one-liners in `~/.claude/settings.json`): finds entries by marker, then compares the full command string. Identical → no-op. Different (e.g. the destructive-git blocker gained a `CLAUDE_BASH_COMMAND / stdin JSON` dual source in 1.9.1) → update in place, preserving `type` and `timeout`, no duplicates.

Both `ig setup` invocations (standalone and post-update) now properly upgrade hooks end-to-end. A dry-run still prints what would change without touching disk.

4 new tests in `src/setup.rs`:
- `test_install_hook_file_identical_is_noop`
- `test_install_hook_file_updates_when_content_differs` (also verifies a `.bak-<ts>` backup is created)
- `test_ensure_hook_registered_identical_is_noop`
- `test_ensure_hook_registered_updates_when_command_differs` (also asserts no duplicate entry)

Test totals: **418** (369 bin + 49 goldens), up from 416 in 1.9.1.

## [1.9.1] — 2026-04-23

### Fixed
- `hooks/ig-guard.sh` (shipped in the binary via `include_str!` and installed by `ig setup`) previously read the command from `$CLAUDE_BASH_COMMAND` only. Claude Code 2.1+ no longer exposes that env var to hooks — the shipped hook silently passed through every command. It now falls back to reading the command from stdin JSON (`.tool_input.command`), identical to the RTK thin-delegator pattern. Existing installs are fixed by re-running `ig setup`.
- Inline one-liner hooks generated by `ig setup` (destructive git blocker, npm/npx blocker) had the same env-var dependency and are now dual-source (env var OR stdin JSON). Re-run `ig setup` to pick up the fixed one-liners in `~/.claude/settings.json`.

## [1.9.0] — 2026-04-23

Full parity with `rtk rewrite` on pipeline handling, env prefix, absolute-path normalization, and git global options — measured in a 4-round × 30-session `claude -p` benchmark (hit rate went from ~8 % of rg/grep attempts rewritten in 1.8.3 to 100 % in 1.9.0, 12× improvement).

### Added

- **Lexer for compound commands** (`src/rewrite.rs`): `rewrite` now splits on top-level shell operators (`|`, `||`, `&&`, `;`) while respecting single and double quotes. Each segment is rewritten independently; for pipelines, only the first segment is touched (stream semantics are preserved for stdin-based downstream filters like `head -20`, `wc -l`, `grep pattern`).
- **Env prefix stripping**: `sudo`, `env`, and repeated `VAR=value` assignments are stripped before classification and re-prepended on the rewritten command (`RUST_LOG=debug rg pat src` → `RUST_LOG=debug IG_COMPACT=1 ig "pat" src`).
- **Absolute binary-path normalization**: `/usr/bin/grep -rn foo src/` is normalized to `grep -rn foo src/` before matching, then rewritten. Same for `/opt/homebrew/bin/rg`, etc.
- **Git global options stripping** (`-C <path>`, `-c <k=v>`, `--git-dir[=…]`, `--work-tree[=…]`, `--no-pager`, `--no-optional-locks`, `--bare`, `--literal-pathspecs`): `git -C /tmp/repo log` → `ig git log`.
- **`dedup_consecutive` filter stage**: new TOML key collapses N consecutive identical output lines into `<line>  (×N)`. Applied early in the pipeline so downstream stages see the deduplicated form. Activated on `docker logs` and `jest` filters.
- **~40 new command categories** routed through the `ig run` filter engine: `make`, `mvn`, `bundle`, `swift`, `mix`, `shellcheck`, `yamllint`, `markdownlint`, `hadolint`, `pre-commit`, `trunk`, `tofu`, `gcloud`, `systemctl`, `ansible-playbook`, `helm` (extended), `pip` (extra), `poetry`, `uv`, `composer`, `brew`, `pio`, `rsync`, `ping`, `next`, `prisma`, `df`, `du`, `ps`, `diff`, `jest`, `playwright`. Total bins covered: **91** (up from ~30).
- **7 new TOML filter files** in `filters/`: `build-tools.toml`, `lint-tools.toml`, `infra-tools.toml`, `pkg-extra.toml`, `net-tools.toml`, `frontend.toml`, `sysinfo.toml`. 42 filter files total.

### Fixed

- **`ls <path>` small-directory regression**: `ls src/` was rewritten to `ig ls src/` and produced more bytes than the native `ls` on short listings. Now `ls <path>` without informative flags (`-l`/`-a`/`-R`) is passthrough; only `ls -la <path>` triggers the rewrite.
- **`ls <glob>` multi-arg crash**: `ls /tmp/*.log` was rewritten to `ig ls /tmp/*.log`; the shell then expanded the glob into N args and `ig ls` errored (accepts one path). Now glob paths (`*`, `?`, `[`) bypass the rewrite.
- **Claude Code 2.1 hook compatibility**: `~/.claude/hooks/ig-guard.sh` previously read `$CLAUDE_BASH_COMMAND` only. Claude Code 2.1.x no longer exposes that env var — the hook now falls back to reading the command from stdin JSON (`.tool_input.command`), matching the RTK thin-delegator pattern.

### Benchmarks — 4 rounds × 30 `claude -p` sessions

| Metric | R1 (hook broken) | R2 (hook BLOCK) | R3 (silent rewrite, pre-lexer) | **R4 (1.9.0)** |
|---|---:|---:|---:|---:|
| `ig` used first | 30 / 30 | 30 / 30 | 30 / 30 | **30 / 30** |
| `rg` fallback attempts | 30 | 22 | 39 | 36 |
| `grep -r` fallback attempts | 6 | 5 | 14 | 16 |
| Pipes with rg/grep | 20 | 23 | 25 | **28** |
| BLOCK errors visible to the model | 0 (broken) | 27 | 0 | **0** |
| Pipelines silently rewritten | 0 | 0 | 0 | **28 / 28** |

### Tests

- **367 bin tests + 49 goldens** pass (was 362 + 43 in 1.8.3) — 11 new tests for pipeline rewrites, env/sudo stripping, absolute paths, git global options, dedup stage, ls glob/small-dir passthrough.

## [1.8.3] — 2026-04-20

### Documentation
- README: new "Compact search mode" section covering `IG_COMPACT=1` and its overrides (`IG_LINE_MAX`, `IG_MAX_MATCHES_PER_FILE`, `IG_MAX_MATCHES_TOTAL`).
- README: `Token Savings` table replaced with real measurements from a Next.js + Symfony monorepo (per-category rows, sparse-vs-dense distinction).
- README: `ig read --plain` documented alongside the existing `-s` / `-a` / `-b` flags.
- New CHANGELOG.md.

## [1.8.2] — 2026-04-20

### Added
- `ig read --plain` / `-p`: output without line-number prefixes — byte-exact with `cat`. The PreToolUse hook now rewrites `cat file` to `ig read --plain file` so the rewrite no longer adds bytes.
- Compact search mode (`IG_COMPACT=1`, auto-set by `grep`/`rg` rewrites):
  - UTF-8-safe line truncation at 100 chars with `…` marker.
  - Per-file match cap (default 10) with `… +N more` footer.
  - Global match cap (default 200) with `… global cap reached` marker.
  - Inter-file blank line and `--` separator between non-contiguous matches are suppressed.
- New `docker-compose-ps` filter — previously `docker compose ps` used the permissive `docker-compose` filter and compressed only −8%.

### Changed
- `rewrite_cat` heuristic: files > 8 KB with a source-code extension (`rs`, `ts`, `tsx`, `js`, `jsx`, `py`, `go`, `php`, `java`, `cpp`, `rb`, …) are rewritten to `ig read <file> -s` (signatures). Small / config / docs files go through `--plain`.
- `rewrite_ls`: bare `ls` is now passthrough. Rewriting added noise on terse native output.
- `filters/docker-logs`: drops `/health` probes and connection banners, `tail=25` (was 50). Compression: −34% → −54%.
- `filters/vitest`: drops `✗ suite summary`, `node_modules/` stack frames, `Start at` and `Duration` lines. −17% regression vs v1.7.1 → −50% gain.
- `filters/phpunit`, `filters/pest`: `drop_lines` removed (mutually exclusive with `keep_lines` in the engine — caused filters to be skipped entirely when combined).

### Fixed
- `cat <file>` rewrite no longer produces output larger than raw `cat` (previously +18–27% due to line-number prefixes).
- `ls` on a small directory no longer regresses to −55% (bare `ls` is now passthrough).
- PHP test filters (`phpunit`, `pest`) no longer emit `warn: skipping filter from builtin: keep_lines and drop_lines are mutually exclusive` and re-apply correctly.

### Benchmarks vs rtk 0.37.1

On dense search patterns, ig now matches or beats `rtk grep --context-only`:

| Pattern | raw | ig compact | rtk ctx |
|---|---:|---:|---:|
| `fn ` (src/) | 58 KB | **−81%** | −81% |
| `Result` (src/) | 31 KB | **−67%** | −68% |
| `struct` (src/) | 11 KB | **−38%** | −25% |
| `impl` (src/) | 4.4 KB | **−21%** | −10% |
| `fn build` (10 matches) | 674 B | **−5%** | −15% *(rtk header overhead)* |

### Tests
- 394 tests pass (was 351 before the refactor).
