# instant-grep (`ig`)

Trigram-indexed regex search CLI in Rust — built for fast agent and editor-adjacent search workflows.

## Repository

https://github.com/MakFly/instant-grep

## Stack

Rust 1.94, edition 2024. Binary: `ig`. Installed at `~/.local/bin/ig`.

## Build & Test

```bash
cargo build --release
cargo test
cp target/release/ig ~/.local/bin/ig
```

## Architecture

Sparse n-grams (port of GitHub Blackbird / danlark1/sparse_ngrams) with covering algorithm. The index lives in the **XDG cache** (`~/.cache/ig/projects/<hash-of-root>/`) by default, not in `<root>/.ig/`. `find_root` recognises `package.json`, `Cargo.toml`, `go.mod`, etc. in addition to `.git/`. Set `IG_LOCAL_INDEX=1` to force local mode.

A **single global daemon** serves every project on the machine via `~/.cache/ig/daemon/daemon.sock`. `GlobalState` holds an `LRU<root, Arc<TenantState>>` (cap 32, override via `IG_DAEMON_TENANTS_MAX`). Each `TenantState` lazily opens its `IndexReader` on first query and keeps per-tenant regex / `NgramQuery` LRU caches.

Cache invalidation uses a 16-byte **seal** file (`generation: u64`, `finalized_at_nanos: u64`) atomic-renamed as the final act of every rebuild. The daemon checks the seal on each query (pull, authoritative) **and** has a `notify` watcher on `.ig/` (push, best-effort). Full contract: `docs/specs/SPEC-daemon-cache-invalidation.md`.

**Cache layout** (v1.19.0+):

```
~/.cache/ig/
├── daemon/         daemon.sock + daemon.pid + daemon.log[.1…5]
├── projects/<hash>/   per-project artifacts (lexicon, postings, metadata, seal, …)
├── by-name/<slug>     human-friendly symlinks → ../projects/<hash>
├── tee/               centralized tee output
└── manifest.json      global registry (cheap cache-ls)
```

`cache::ensure_layout()` migrates pre-v1.19 installs (hash dirs at root, daemon files mixed in) on first launch. Idempotent, lockfile-protected.

**Setup managed-block** (v1.19.1+): `ig setup` writes a sentinel-wrapped section into `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, etc. — automatically refreshed on every `ig update` (quiet by default, only drift is reported). The deep-dive rules file `~/.claude/rules/tools/ig.md` is fully owned by `ig setup` (overwritten on every run).

Pipeline: `regex → regex-syntax Extractor → covering sparse n-grams → hash table lookup (lexicon.bin) → vbyte-decoded posting list intersection (postings.bin) → bloom/loc/zone mask filter → parallel regex verification`

## Key files

### Index core
- `src/index/ngram.rs` — sparse n-gram algorithm (hash_bigram, build_all_ngrams, build_covering_ngrams) + `NgramMaskEntry` type alias.
- `src/index/writer.rs` — index build pipeline. `build_index` (full) and `incremental_overlay` both call `seal::bump_seal` as their last act.
- `src/index/reader.rs` — index query (mmap + hash table). Uses bloom_mask / loc_mask / zone_mask from `PostingEntry` for sub-trigram filtering.
- `src/index/vbyte.rs` — varbyte posting codec, `PostingEntry` with masks (v1.17.1).
- `src/index/seal.rs` — 16-byte atomic publish marker (v1.18.0).
- `src/index/overlay.rs` — incremental overlay reader/writer + tombstones.
- `src/index/merge.rs` — k-way merge with atomic tmp+rename publish for `lexicon.bin` and `postings.bin`.
- `src/index/metadata.rs` — `IndexMetadata` (file_count, ngram_count, files…). `INDEX_VERSION = 13`. Atomic write via tmp+rename.
- `src/query/extract.rs` — regex → `NgramQuery` conversion + `regex_to_query_costed` cost-estimation closure.
- `src/cache.rs` — XDG cache layout, `gc`, `migrate`, `cache-ls` (v1.15.0).

### Daemon
- `src/daemon.rs` — single global Unix-socket server. `GlobalState` (multi-tenant LRU) + `ActiveProject` (per-project source-file watcher + `.ig/` seal watcher).

### CLI / agent integration
- `src/read.rs` — smart file reading (full + signatures-only mode).
- `src/smart.rs` — 2-line heuristic file summaries.
- `src/pack.rs` — project context generator (`.ig/context.md`).
- `src/ls.rs` — compact directory listing.
- `src/rewrite.rs` — command rewriting engine for PreToolUse hook.
- `src/runner.rs` — `ig run`/`ig proxy` command proxy with filter pipeline and tee fallback.
- `src/tee.rs` — tee store for raw output of truncated/failed commands (`.ig/tee/`).
- `src/filter/` — TOML-driven filter pipeline (8 stages: ansi, replace, keep, drop, truncate, head, tail, fallback).
- `src/tracking.rs` — token savings tracking (JSONL history).
- `src/gain.rs` — savings dashboard.
- `src/setup.rs` — AI agent auto-configuration + hook installation (self-updating shell-hook block, v1.17.0).
- `src/update.rs` — `ig warm`, `ig projects {list,forget}` (v1.17.0).

## Commands

```
ig "pattern" [path]          # search (shortcut, recommended)
ig search <pattern> [path]   # search (explicit)
ig index [path]              # build/rebuild index
ig status [path]             # show stats
ig watch [path]              # auto-rebuild on file changes
ig warm [path]               # warm a project with the global daemon (v1.17.0)
ig projects list             # list active (warmed) projects (v1.17.0)
ig projects forget <root>    # drop a project from the active set (v1.17.0)
ig daemon start              # start the global daemon (v1.16.0+)
ig daemon stop               # stop the daemon
ig daemon status             # daemon PID + socket
ig daemon install            # systemd-user (Linux) or launchd (macOS) auto-start
ig daemon uninstall          # remove auto-restart
ig query <pattern> [path]    # query daemon directly
ig gc [--days N] [--dry-run] # prune stale XDG cache entries (v1.15.0)
ig migrate [--dry-run]       # move <root>/.ig/ to ~/.cache/ig/ (v1.15.0)
ig cache-ls                  # list cache entries with size + last_used (v1.15.0)
ig files [path]              # list project files
ig symbols [path]            # extract symbol definitions
ig context <file> <line>     # show enclosing code block
ig ls [path]                 # compact directory listing
ig read <file> [-s]          # smart file reading (signatures mode)
ig smart [path]              # 2-line file summaries
ig pack [path]               # generate .ig/context.md
ig gain [--clear]            # token savings dashboard
ig run <cmd>                 # run any command through the filter pipeline
ig proxy <cmd>               # alias of `ig run` (more intuitive in hook rewrites)
ig tee list                  # list saved raw outputs of truncated / failed commands
ig tee show <id>             # print the raw output of a tee entry
ig tee clear                 # delete every tee entry
ig rewrite <cmd>             # rewrite command to ig equivalent (hook-internal, hidden from --help)
ig completions <shell>       # generate shell completions
ig setup                     # configure AI CLI agents + install hooks
```

## Conventions

- `bun` as package manager (N/A for Rust, but keep for any JS tooling)
- Conventional Commits in English
- INDEX_VERSION must be bumped when on-disk format changes
- Tests must reproduce danlark1 test vectors for sparse n-grams
- 38 default excluded directories (node_modules, target, vendor, etc.)

## Testing policy — always run REAL tests, not just unit tests

Unit tests catch logic bugs but they don't catch:
- File-system layout changes (migrations, atomic-rename races)
- Daemon socket / PID / log path drift
- Cross-process interactions (writer rebuilds while daemon serves queries)
- macOS-specific behavior (FSEvents reliability, codesign, mmap survival across truncate)

So before declaring any work done, run **all three layers**:

1. **Unit tests** — `cargo test --quiet`. 425+ passing, no failures.
2. **Lint + format** — `cargo clippy --all-targets -- -D warnings && cargo fmt --check`.
3. **Real tests** — exercise the actual binary against the actual cache:
   - `cp target/release/ig ~/.local/bin/ig && codesign -fs - ~/.local/bin/ig` (macOS).
   - `ig daemon stop && ig daemon start` — verify the daemon comes up cleanly.
   - `ig daemon status` — confirms PID + socket path.
   - On a real project (tilvest, instant-grep, …) : `ig -c "<pattern>"` returns the same count as `rg -c "<pattern>"` (parity check).
   - Inspect `~/Library/Caches/ig/` (or `~/.cache/ig/` on Linux) to confirm the on-disk layout matches expectations.

A change that passes unit tests but breaks real-world use (daemon hangs, cache layout corrupt, codesign rejected) is **not done**. Skip the real tests at your own risk — the v1.17.x daemon stale-state bug shipped because real tests weren't run between code change and CI green.

## Filter matching policy

`ig run <cmd>` looks up a filter with a two-step lookup in `src/cmds/run.rs::resolve_filter`:
1. Try the raw command string (`cargo test --release`).
2. On miss, retry with `args[0]` replaced by its basename (`/usr/bin/cargo` → `cargo`).

This is how filters whose `match` regex starts with `^pytest` still activate when the command is invoked through an absolute path (shebang, wrapper, mock). Do not add path-aware regexes to filter `.toml` files — the normalization does that for you.

`ig run` also transparently routes to dedicated ig subcommands when appropriate:
- `ig run ls …` → `ig ls`
- `ig run git status/log/diff` → `ig git`
- `ig run find …` → `ig files`
- `ig run cat …` → `ig read`

Routing is opt-out via `IG_RUN_ROUTE=0`.
