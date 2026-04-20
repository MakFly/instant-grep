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

Sparse n-grams (port of GitHub Blackbird / danlark1/sparse_ngrams) with covering algorithm. Index stored in `.ig/` at project root (lexicon.bin + postings.bin + metadata.bin, all mmap'd).

Pipeline: `regex → regex-syntax Extractor → covering sparse n-grams → hash table lookup → posting list intersection → parallel regex verification`

## Key files

- `src/index/ngram.rs` — core algorithm (hash_bigram, build_all_ngrams, build_covering_ngrams)
- `src/index/writer.rs` — index build pipeline (also generates tree.txt + context.md)
- `src/index/reader.rs` — index query (mmap + hash table)
- `src/query/extract.rs` — regex → NgramQuery conversion
- `src/daemon.rs` — Unix socket daemon for sub-ms queries
- `src/read.rs` — smart file reading (full + signatures-only mode)
- `src/smart.rs` — 2-line heuristic file summaries
- `src/pack.rs` — project context generator (.ig/context.md)
- `src/ls.rs` — compact directory listing
- `src/rewrite.rs` — command rewriting engine for PreToolUse hook
- `src/runner.rs` — `ig run`/`ig proxy` command proxy with filter pipeline and tee fallback
- `src/tee.rs` — tee store for raw output of truncated/failed commands (`.ig/tee/`)
- `src/filter/` — TOML-driven filter pipeline (8 stages: ansi, replace, keep, drop, truncate, head, tail, fallback)
- `src/tracking.rs` — token savings tracking (JSONL history)
- `src/gain.rs` — savings dashboard
- `src/setup.rs` — AI agent auto-configuration + hook installation

## Commands

```
ig "pattern" [path]          # search (shortcut, recommended)
ig search <pattern> [path]   # search (explicit)
ig index [path]              # build/rebuild index
ig status [path]             # show stats
ig watch [path]              # auto-rebuild on file changes
ig daemon start [path]       # start daemon
ig daemon stop [path]        # stop daemon
ig daemon status [path]      # check status
ig daemon install [path]     # auto-restart on reboot (macOS)
ig daemon uninstall [path]   # remove auto-restart
ig query <pattern> [path]    # query daemon
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
