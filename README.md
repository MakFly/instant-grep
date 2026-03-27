<p align="center">
  <h1 align="center">instant-grep</h1>
  <p align="center">
    <strong>Trigram-indexed regex search for AI agents and humans</strong>
  </p>
  <p align="center">
    <a href="#installation">Installation</a> &middot;
    <a href="#usage">Usage</a> &middot;
    <a href="#how-it-works">How it works</a> &middot;
    <a href="#benchmarks">Benchmarks</a> &middot;
    <a href="#agent-integration">Agent Integration</a>
  </p>
</p>

---

**instant-grep** (`ig`) is a regex search tool that builds a persistent index of your codebase using [sparse n-grams](https://github.com/danlark1/sparse_ngrams), the same algorithm behind [GitHub Code Search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/) and [Cursor's fast regex search](https://cursor.com/blog/fast-regex-search).

Instead of scanning every file on each search (like `grep` or `ripgrep`), `ig` narrows candidates through an inverted index, then runs regex verification only on matching files. The result: **~3ms searches** on typical projects, **~3.5x faster than ripgrep**.

```
$ ig "async fn.*Result" src/ --stats

src/daemon.rs
23:    pub async fn handle_connection(stream: UnixStream) -> Result<()> {

src/index/writer.rs
27:    pub async fn build_index(root: &Path) -> Result<IndexMetadata> {

--- stats ---
Candidates: 4/1284 files (0.3%)
Search: 1.5ms
Index: yes
```

## Why

AI agents (Claude Code, Codex, Cursor) call grep constantly. On large codebases, each `rg` invocation scans every file — 15+ seconds on monorepos. This breaks the agent loop, wastes tokens on retries, and increases hallucination risk.

`ig` solves this by maintaining a persistent search index. First search auto-builds the index; subsequent searches are near-instant.

|             | ripgrep   | ig (CLI)       | ig (daemon)        |
| ----------- | --------- | -------------- | ------------------ |
| 11,350 files | ~34ms    | **~29ms**      | **~0.2ms**         |
| Approach    | Full scan | Index + verify | Persistent process |

> Measured with `time` on a large multi-language project (11,350 source files, default exclusions).
> ig is consistently faster than ripgrep on indexed projects. The daemon eliminates process startup entirely.

## Installation

### One-liner (recommended)

Download the prebuilt binary for your platform (Linux x86_64, macOS x86_64, macOS ARM):

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

> This installs the binary and automatically runs `ig setup` to configure your AI agents (Claude Code, Codex).

This installs `ig` to `~/.local/bin/`. Set a custom directory with `IG_INSTALL_DIR`:

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | IG_INSTALL_DIR=/usr/local/bin bash
```

### Homebrew (macOS / Linux)

```bash
brew install MakFly/tap/ig
```

### Download binary manually

Grab the latest binary from [Releases](https://github.com/MakFly/instant-grep/releases):

| Platform                | Binary             |
| ----------------------- | ------------------ |
| Linux x86_64            | `ig-linux-x86_64`  |
| Linux ARM64             | `ig-linux-aarch64` |
| macOS x86_64            | `ig-macos-x86_64`  |
| macOS ARM (M1/M2/M3/M4) | `ig-macos-aarch64` |

```bash
# Example: Linux
curl -fsSL https://github.com/MakFly/instant-grep/releases/latest/download/ig-linux-x86_64 -o ~/.local/bin/ig
chmod +x ~/.local/bin/ig
```

```bash
# Verify installation
ig --version
```

### Upgrade

Re-run the install script (same as initial install):
```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

Or with Homebrew: `brew upgrade ig`

### Uninstall

```bash
rm ~/.local/bin/ig          # remove binary
rm -rf /path/to/project/.ig # remove project indexes
# To undo ig setup changes, remove the ig-rewrite.sh hook entry
# from ~/.claude/settings.json and the Search Tools section from ~/.claude/CLAUDE.md
```

> **Windows:** Not directly supported. Use WSL2 (Windows Subsystem for Linux) with the Linux binary.

### Build from source

Requires Rust 1.75+ (tested on 1.94):

```bash
git clone https://github.com/MakFly/instant-grep.git
cd instant-grep
cargo build --release
cp target/release/ig ~/.local/bin/
```

## Usage

### Search

```bash
# Search with auto-indexing (builds .ig/ on first run)
ig "pattern" .

# Case-insensitive
ig -i "todo|fixme" .

# Filter by file type
ig "useRouter" . --type ts

# Context lines (like grep -C)
ig -C 3 "async fn" src/

# JSON output (for AI agents)
ig "fetchData" . --json

# Show performance stats
ig "Result<T>" . --stats

# Force brute-force (skip index)
ig "pattern" . --no-index
```

### Index management

```bash
# Build or rebuild index
ig index .

# Show index info
ig status .

# Watch for changes and auto-rebuild
ig watch .
```

### Daemon mode (sub-millisecond)

For agents that call search in a tight loop:

```bash
# Start daemon in background
ig daemon start .

# Query via Unix socket — 0.2ms response
ig query "pattern" .

# Status / stop
ig daemon status .
ig daemon stop .

# Auto-restart on macOS reboot (launchd)
ig daemon install .
ig daemon uninstall .
```

### Explore codebase

```bash
# List all project files (respects .gitignore)
ig files .
ig files -t rust              # only Rust files
ig files --json               # JSON output for agents

# Compact directory listing (token-optimized)
ig ls                         # dirs grouped, files with sizes
ig ls src/                    # specific directory

# Read files with smart filtering
ig read src/main.rs           # numbered lines
ig read src/main.rs -s        # signatures + imports only (2x fewer tokens)

# 2-line smart summary per file
ig smart                      # all files in project
ig smart src/                 # specific directory
ig smart src/main.rs          # single file

# Extract symbol definitions
ig symbols .                  # all functions/classes/structs
ig symbols -t ts              # only TypeScript symbols

# Show full code block at a specific line
ig context src/main.rs 42     # shows the enclosing function/class

# Generate project context for AI agents
ig pack                       # generates .ig/context.md (tree + summaries)
```

### Token savings

`ig` tracks how many bytes it saves vs. raw `cat`/`ls`/`grep` output:

```bash
ig gain                       # show savings dashboard
ig gain --clear               # reset history
```

### Shell completions

```bash
ig completions zsh > ~/.zsh/completions/_ig
ig completions bash > ~/.bash_completion.d/ig
ig completions fish > ~/.config/fish/completions/ig.fish
```

### AI agent setup

```bash
# Auto-configure Claude Code, Codex, Gemini CLI to use ig
ig setup
```

`ig setup` automatically:
- Adds `Bash(ig *)` permission to Claude Code
- Installs a `PreToolUse` hook that rewrites `cat`/`grep`/`ls`/`tree`/`find` → `ig` equivalents
- Adds search instructions to `CLAUDE.md`

### All flags

```
ig <PATTERN> [PATH]              # shortcut (recommended)
ig search <PATTERN> [PATH]       # explicit subcommand (also works)
  -i, --ignore-case            Case-insensitive
  -A, --after-context <N>      Lines after match
  -B, --before-context <N>     Lines before match
  -C, --context <N>            Lines before + after
  -c, --count                  Match count per file
  -l, --files-with-matches     File paths only
  -t, --type <TYPE>            Filter: rs, ts, py, go, php, etc.
  -g, --glob <GLOB>            Filter: "*.tsx", "*.go"
      --json                   JSON lines output
      --stats                  Show candidate ratio + timing
      --no-index               Skip index, brute-force scan
      --no-default-excludes    Include node_modules, target, etc.
      --max-file-size <BYTES>  Override 1MB default limit
  -w, --word-regexp            Match whole words only
  -F, --fixed-strings          Treat pattern as literal (not regex)

ig ls [PATH]                     # compact directory listing
ig read <FILE> [-s|--signatures] # read file (signatures-only mode)
ig smart [PATH]                  # 2-line file summaries
ig pack [PATH]                   # generate .ig/context.md
ig files [PATH]                  # list project files
ig symbols [PATH]                # extract symbol definitions
ig context <FILE> <LINE>         # show enclosing code block
ig gain [--clear]                # token savings dashboard
ig completions <SHELL>           # generate shell completions
ig setup                         # configure AI CLI agents
```

## How it works

### The pipeline

```
regex pattern
    │
    ▼
regex-syntax::Extractor → extract literal sequences
    │
    ▼
build_covering_ngrams() → minimal set of sparse n-grams
    │
    ▼
FNV-1a hash → NgramKey (u64) → lookup in mmap'd hash table
    │
    ▼
intersect posting lists → candidate file IDs
    │
    ▼
parallel regex verification (rayon) → only on candidates
    │
    ▼
results (colored / JSON)
```

### Sparse n-grams

Traditional trigram indexes use fixed 3-character windows. `ig` uses **variable-length sparse n-grams** based on [danlark1/sparse_ngrams](https://github.com/danlark1/sparse_ngrams) (the algorithm behind GitHub Code Search):

- **Bigram hash weighting** — each character pair gets a deterministic weight via a Murmur2-like hash
- **Monotonic stack** — n-gram boundaries are placed where boundary weights exceed all interior weights
- **Covering algorithm** — at query time, only the minimal set of n-grams needed to guarantee a match is extracted

This produces fewer, more selective keys than trigrams. For the pattern `"fetchSellerListingsAction"`:

```
Trigrams:     23 keys → 47 candidate files
Sparse grams:  3 keys →  4 candidate files (12x better)
```

### On-disk format (v7)

The index lives in `.ig/` at the project root:

| File           | Format                                               | Size (1,552 files) |
| -------------- | ---------------------------------------------------- | ------------------ |
| `metadata.bin` | bincode — file paths, mtimes, git SHA                | 111 KB             |
| `lexicon.bin`  | Hash table: `[NgramKey:u64, offset:u32, byte_len:u32]` | 31 MB           |
| `postings.bin` | Delta + VByte encoded posting lists, concatenated    | 7.1 MB             |

The lexicon is memory-mapped (`mmap`). Postings are memory-mapped and VByte-compressed (~50-60% smaller than raw u32). Metadata is deserialized via `bincode` (~1ms).

**Overlay index** (incremental updates): when <100 files change, ig writes `overlay.bin` + `overlay_lex.bin` + `tombstones.bin` instead of rebuilding. Query-time merge is transparent.

**Streaming SPIMI pipeline**: files are processed in batches of 1,000 (parallel rayon read + ngram extraction per batch). Each batch's ngrams are fed into a bounded-memory accumulator (128MB budget), flushed to disk segments when full, then the batch is freed. The lexicon hash table is written directly via mmap (no heap allocation). This keeps RAM proportional to the number of unique n-grams, not the number of files.

### Default exclusions

38 directories are excluded by default (override with `--no-default-excludes`):

`node_modules` `target` `dist` `build` `.next` `.nuxt` `__pycache__` `.venv` `venv` `vendor` `.git` `.hg` `.svn` `coverage` `.cache` `.turbo` `.output` `.vercel` `tmp` `.temp` `.gradle` `.idea` `.vscode` `.terraform` `.pants.d` `bazel-out` `.mypy_cache` `.ruff_cache` `.pytest_cache` `.tox` `bower_components` `.dart_tool` `.pub-cache` `.cargo` `Pods` and more.

Files larger than 1 MB are also skipped by default (`--max-file-size` to override).

## Benchmarks

Measured on two multi-language projects (11K and 3K source files). Best of 3 runs. Apple M4 Max, macOS 15.5, ripgrep 15.1.

### CLI: ig v1.3.0 vs ripgrep

Wall time includes process startup (~15ms on macOS). Best of 3 runs.

| Pattern | ig v1.3.0 | ripgrep 15.1 | Winner |
|---------|-----------|-------------|--------|
| `function` (11K files) | **33ms** | 39ms | ig 1.2x |
| `class\s+\w+` (11K files) | **29ms** | 34ms | ig 1.2x |
| `deprecated` (11K files) | **21ms** | 31ms | ig 1.5x |
| no-match (11K files) | **20ms** | 30ms | ig 1.5x |
| `import` (11K files) | **24ms** | 32ms | ig 1.3x |
| `function` (3K files) | **40ms** | 43ms | ig 1.1x |
| `class\s+\w+` (3K files) | **33ms** | 44ms | ig 1.3x |
| `deprecated` (3K files) | **22ms** | 37ms | ig 1.7x |

> ig v1.1.0 wins on **all** patterns. The escape hatch threshold was raised from 60% to 85%, so common patterns like `"function"` no longer trigger brute-force fallback. Single-file search is also supported: `ig "pattern" specific-file.rs`.

### Daemon mode: actual search time (1,552 files)

The daemon keeps the index in memory. These are the **server-side search times** (no process startup), extracted from JSON responses:

| Query                      | Candidates   | Search time |
| -------------------------- | ------------ | ----------- |
| `"DistributionController"` | 2 / 1,552    | **0.26ms**  |
| `"function"`               | 974 / 1,552  | 17.54ms     |
| `"exception"`              | 39 / 1,552   | **1.54ms**  |
| `"ZZZNOTFOUND"`            | 0 / 1,552    | **0.02ms**  |
| `"middleware"`              | 30 / 1,552   | **1.22ms**  |
| `"Route::"`                | 4 / 1,552    | **0.18ms**  |
| `"class "`                 | 897 / 1,552  | 11.36ms     |

> Rare patterns with few candidates: **sub-millisecond**. Common patterns touching hundreds of files are slower due to regex verification on each candidate.

### Index build performance

| Operation            | Time   | Notes                                      |
| -------------------- | ------ | ------------------------------------------ |
| Fresh build          | 480ms  | 1,552 files, SPIMI streaming, 2 segments   |
| Incremental (no-op)  | 28ms   | Git diff detects no changes                |
| Index size           | 36 MB  | lexicon 31MB + postings 7MB + metadata 111KB |
| Peak RAM (1.5K files)| 440 MB | Streaming batch + mmap lexicon             |
| Peak RAM (92K files) | 6.8 GB | Down from 17.9 GB pre-streaming (-62%)     |

### Scaling curve — ig gets faster on larger projects

Measured on v1.0.0. v1.1.0 improves common-pattern performance further (escape hatch threshold raised to 85%).

| Project | Files | ig search | rg search | Speedup | Build RSS |
|---------|------:|----------|----------|---------|----------|
| laravel-app | 49 | 19ms | 21ms | 1.1x | 28 MB |
| distribution-app | 1,552 | 70ms | 33ms | 0.5x | 440 MB |
| **Next.js** | **24,760** | **627ms** | **1,490ms** | **2.4x** | 860 MB |
| **Linux kernel** | **92,585** | **1,290ms** | **5,119ms** | **4.0x** | 6.8 GB |

> On the Linux kernel (92K files), a zero-result search takes **28ms with ig vs 5,279ms with rg** — a **189x speedup**.

### Daemon latency distribution (1,001 queries, p50/p95/p99)

| Metric | Value |
|--------|-------|
| p50 | **0.71ms** |
| p95 | 4.51ms |
| p99 | 4.69ms |
| Throughput | **312 QPS** (effective) / **2,695 QPS** (server-side) |

### Overlay — incremental rebuild

| Changed files | Time | vs full rebuild |
|--------------|------|----------------|
| 0 (no-op) | 28ms | — |
| 1-100 files | 28-91ms | **6x faster** |
| 1,552 (all) | 568ms | full SPIMI |

### ig vs ripgrep — when to use which

|          | ig                                                            | ripgrep                                      |
| -------- | ------------------------------------------------------------- | -------------------------------------------- |
| Best at  | Projects with persistent index, agent loops, repeated queries | One-off searches, no setup, cold filesystems |
| Weakness | Process startup (~15ms), short patterns fall back to brute    | Scans all files every time, no daemon mode   |

> **Honest note:** On small projects (<100 files), both tools are equally fast (~20ms, dominated by process startup). ig's advantage shows on **large projects** (2.4-189x faster on 25K-92K files) and on **repeated queries** (daemon mode: sub-ms with rayon-parallel verification). In v1.1.0, the escape hatch threshold was raised from 60% to 85%, so common patterns like `"function"` no longer fall back to brute-force — ig now wins on all tested patterns. Short patterns (<3 chars) use a bigram-indexed fallback instead of full brute-force. See [full benchmark report](benchmarks/REPORT.md) for details.

## Agent Integration

`ig` is designed as a **CLI tool for AI agents**, following the [CLI > MCP consensus](https://ejholmes.github.io/2026/02/28/mcp-is-dead-long-live-the-cli.html):

- **35x fewer tokens** than MCP (4K vs 145K for equivalent tool schemas)
- **Zero config** — just `ig --json` via Bash
- **LLMs already know CLIs** — trained on millions of man pages and READMEs
- **Composable** — pipe to `jq`, `head`, `wc`, other tools

### Claude Code

```bash
# Claude Code calls this via the Bash tool:
ig "fetchSellerListings" /path/to/project --json
```

Output:

```json
{"file":"src/actions/seller.ts","line":30,"text":"export async function fetchSellerListingsAction("}
{"_stats":{"candidates":4,"total":1284,"search_ms":1.5,"used_index":true}}
```

### Daemon for high-frequency agents

```bash
# Start once per project
ig daemon /path/to/project &

# Agent queries via Unix socket — 0.2ms per query
ig query "useRouter" /path/to/project
```

### Token optimization (ig vs RTK vs baseline)

`ig` reduces token consumption for AI agents through smart file reading, compact directory listings, and pre-generated project context. Measured on a 1,285-file Next.js project with `claude -p`:

| Approach | Time | Turns | Cost | vs Baseline |
|----------|-----:|------:|-----:|-------------|
| **ig** (context.md + ls + smart) | **18s** | **4** | **$0.15** | 2.5x faster, 51% cheaper |
| **RTK** (ls + smart + read) | 19s | 4 | $0.15 | 2.3x faster, 50% cheaper |
| Baseline (ls + cat + tree) | 45s | 2 | $0.30 | — |

Key features:
- **`ig pack`** generates `.ig/context.md` — a compact project map (tree + file summaries + public APIs) that agents read in a single call instead of 10+ shell commands
- **`ig read --signatures`** shows only imports and function signatures (2x fewer bytes than `cat`)
- **`ig ls`** produces a compact directory listing (81% fewer bytes than `ls -la`)
- **`ig rewrite`** + PreToolUse hook transparently intercepts `cat`/`grep`/`ls`/`tree`/`find` and redirects to `ig` equivalents
- **`ig gain`** shows a savings dashboard (bytes saved per command)

### Compared to alternatives

|                      | ig    | RTK   | ripgrep | MCP grep server  |
| -------------------- | ----- | ----- | ------- | ---------------- |
| Index-based search   | Yes   | No    | No      | No               |
| Search latency       | 1.5ms | N/A   | ~95ms   | ~95ms + overhead |
| Token optimization   | Yes   | Yes   | No      | No               |
| Project context pack | Yes   | No    | No      | No               |
| Command rewriting    | Code read/search | All CLI | No | No          |
| Token cost (schema)  | 4K    | 4K    | 4K      | 145K             |
| Daemon mode          | Yes   | No    | No      | No               |

> ig and RTK are complementary: ig optimizes code reading and search, RTK optimizes git, npm, cargo, docker output. Both use the same Claude Code hook protocol.

## Architecture

```
ig
├── index/
│   ├── ngram.rs      — Sparse n-gram extraction (port of danlark1/sparse_ngrams)
│   ├── vbyte.rs      — Delta + VByte codec for posting list compression
│   ├── spimi.rs      — SPIMI segment builder (bounded-memory accumulation)
│   ├── merge.rs      — K-way merge of segments + lexicon hash table builder
│   ├── overlay.rs    — Incremental overlay index + tombstone bitmap
│   ├── writer.rs     — Index build: SPIMI pipeline + overlay path
│   ├── reader.rs     — Index query: mmap lexicon + VByte postings + overlay merge
│   ├── postings.rs   — Sorted merge intersection/union
│   └── metadata.rs   — Binary + JSON metadata (bincode)
├── query/
│   ├── extract.rs    — Regex → NgramQuery (via regex-syntax Extractor + covering algo)
│   └── plan.rs       — NgramQuery { And, Or, Ngram, All }
├── search/
│   ├── indexed.rs    — Full pipeline: query → candidates → parallel verify
│   ├── fallback.rs   — Brute-force scan (no index)
│   └── matcher.rs    — File-level regex matching + line extraction
├── context.rs        — Code block extraction
├── symbols.rs        — Symbol definition extraction (multi-language)
├── read.rs           — Smart file reading (full + signatures-only mode)
├── smart.rs          — 2-line heuristic file summaries
├── pack.rs           — Project context generator (.ig/context.md)
├── ls.rs             — Compact directory listing
├── rewrite.rs        — Command rewriting engine (cat→ig read, grep→ig, etc.)
├── tracking.rs       — Token savings tracking (JSONL history)
├── gain.rs           — Savings dashboard
├── setup.rs          — AI agent auto-configuration + hook installation
├── update.rs         — Background update checker
├── daemon.rs         — Unix socket server + client + start/stop/install lifecycle
├── watch.rs          — File watcher (notify crate) + auto-rebuild
└── walk.rs           — Gitignore-aware file walking + 38 default exclusions
```

## Credits

- [danlark1/sparse_ngrams](https://github.com/danlark1/sparse_ngrams) — the sparse n-gram algorithm (C++), ported to Rust
- [Cursor — Fast regex search](https://cursor.com/blog/fast-regex-search) — the inspiration for this project
- [Russ Cox — Regular Expression Matching with a Trigram Index](https://swtch.com/~rsc/regexp/regexp4.html) — foundational algorithm
- [GitHub — The technology behind code search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/) — sparse n-gram architecture reference
- [BurntSushi](https://github.com/BurntSushi) — `regex-syntax`, `ignore`, `memchr` crates that power the Rust ecosystem

## License

MIT
