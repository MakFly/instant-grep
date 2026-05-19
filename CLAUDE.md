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

Sparse n-grams (port of GitHub Blackbird / danlark1/sparse_ngrams) with covering algorithm.

**Process-per-invocation** (v2.0.0+): every `ig` command is a one-shot process. There is no daemon, no Unix socket, no background watcher. On each query the binary opens the on-disk index, serves the search, and exits. If the index is missing or stale (`INDEX_VERSION` bump, source files newer than metadata), the search subcommand rebuilds it inline before serving the first results.

The index lives in the **XDG cache** (`~/.cache/ig/projects/<hash-of-root>/`) by default, not in `<root>/.ig/`. `find_root` recognises `package.json`, `Cargo.toml`, `go.mod`, etc. in addition to `.git/`. Set `IG_LOCAL_INDEX=1` to force local mode.

**Cache layout** (v2.0.0+):

```
~/.cache/ig/
├── projects/<hash>/   per-project artifacts (lexicon, postings, metadata, …)
├── by-name/<slug>     human-friendly symlinks → ../projects/<hash>
├── tee/               centralized tee output
└── manifest.json      global registry (cheap cache-ls)
```

`cache::ensure_layout()` migrates pre-v1.19 installs (hash dirs at root) on first launch. Idempotent, lockfile-protected. The legacy `daemon/` directory (sock + pid + rotated logs) is also pruned by `ig update` after the v1 → v2 upgrade.

**Setup managed-block**: `ig setup` writes a sentinel-wrapped section into `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, etc. — automatically refreshed on every `ig update` (quiet by default, only drift is reported). The deep-dive rules file `~/.claude/rules/tools/ig.md` is fully owned by `ig setup` (overwritten on every run).

Pipeline: `regex → regex-syntax Extractor → covering sparse n-grams → hash table lookup (lexicon.bin) → vbyte-decoded posting list intersection (postings.bin) → bloom/loc/zone mask filter → parallel regex verification`

## Key files

### Index core
- `src/index/ngram.rs` — sparse n-gram algorithm (hash_bigram, build_all_ngrams, build_covering_ngrams) + `NgramMaskEntry` type alias.
- `src/index/writer.rs` — index build pipeline (`build_index` full + `incremental_overlay`).
- `src/index/reader.rs` — index query (mmap + hash table). Uses bloom_mask / loc_mask / zone_mask from `PostingEntry` for sub-trigram filtering.
- `src/index/vbyte.rs` — varbyte posting codec, `PostingEntry` with masks (v1.17.1).
- `src/index/overlay.rs` — incremental overlay reader/writer + tombstones.
- `src/index/merge.rs` — k-way merge with atomic tmp+rename publish for `lexicon.bin` and `postings.bin`.
- `src/index/metadata.rs` — `IndexMetadata` (file_count, ngram_count, files…). Atomic write via tmp+rename.
- `src/query/extract.rs` — regex → `NgramQuery` conversion + `regex_to_query_costed` cost-estimation closure.
- `src/cache.rs` — XDG cache layout, `gc`, `migrate`, `cache-ls`.

### CLI / agent integration
- `src/read.rs` — smart file reading (full + signatures-only mode).
- `src/smart.rs` — 2-line heuristic file summaries.
- `src/pack.rs` — project context generator.
- `src/ls.rs` — compact directory listing.
- `src/rewrite.rs` — command rewriting engine for PreToolUse hook.
- `src/runner.rs` — `ig run`/`ig proxy` command proxy with filter pipeline and tee fallback.
- `src/tee.rs` — tee store for raw output of truncated/failed commands.
- `src/filter/` — TOML-driven filter pipeline.
- `src/tracking.rs` — token savings tracking (JSONL history).
- `src/gain.rs` — savings dashboard.
- `src/setup.rs` — AI agent auto-configuration + hook installation.
- `src/update.rs` — self-update + legacy cleanup (daemon sock/pid/log pruning).

## Commands

```
ig "pattern" [path]          # search (shortcut, recommended)
ig search <pattern> [path]   # search (explicit)
ig index [path]              # build/rebuild index
ig status [path]             # show stats
ig gc [--days N] [--max-size 5GB] [--dry-run] # prune stale / oversized XDG cache
ig migrate [--dry-run]       # move <root>/.ig/ to ~/.cache/ig/
ig cache-ls                  # list cache entries with size + last_used
ig files [path]              # list project files
ig symbols [path]            # extract symbol definitions
ig context <file> <line>     # show enclosing code block
ig ls [path]                 # compact directory listing
ig read <file> [-s]          # smart file reading (signatures mode)
ig smart [path]              # 2-line file summaries
ig pack [path]               # generate project context
ig gain [--clear]            # token savings dashboard
ig run <cmd>                 # run any command through the filter pipeline
ig proxy <cmd>               # alias of `ig run`
ig tee list|show|clear       # raw output store for truncated / failed commands
ig rewrite <cmd>             # rewrite command to ig equivalent (hook-internal, hidden)
ig completions <shell>       # generate shell completions
ig setup                     # configure AI CLI agents + install hooks
ig update                    # self-update + clean up legacy daemon artifacts
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
- Cross-process interactions (two concurrent `ig index` invocations)
- macOS-specific behavior (codesign, mmap survival across truncate)

So before declaring any work done, run **all three layers**:

1. **Unit tests** — `cargo test --quiet`.
2. **Lint + format** — `cargo clippy --all-targets -- -D warnings && cargo fmt --check`.
3. **Real tests** — exercise the actual binary against the actual cache:
   - `cp target/release/ig ~/.local/bin/ig && codesign -fs - -i dev.makfly.ig ~/.local/bin/ig` (macOS). The `-i dev.makfly.ig` keeps the codesign identifier stable across rebuilds, so TCC (privacy database) and BTM (background-item service) don't re-prompt the user for file-access permissions on every binary change.
   - On a real project (tilvest, instant-grep, …) : `ig -c "<pattern>"` returns the same count as `rg -c "<pattern>"` (parity check).
   - Inspect `~/Library/Caches/ig/` (or `~/.cache/ig/` on Linux) to confirm the on-disk layout matches expectations.

A change that passes unit tests but breaks real-world use (cache layout corrupt, codesign rejected) is **not done**.

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
