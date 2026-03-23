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
| 1,284 files | ~11ms     | **~3ms**       | **~0.2ms**         |
| Approach    | Full scan | Index + verify | Persistent process |

> Measured with `time` on a Next.js project (1,284 source files, default exclusions).
> ig is ~3.5x faster than ripgrep on indexed projects. The daemon eliminates process startup entirely.

## Installation

### One-liner (recommended)

Download the prebuilt binary for your platform (Linux x86_64, macOS x86_64, macOS ARM):

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

This installs `ig` to `~/.local/bin/`. Set a custom directory with `IG_INSTALL_DIR`:

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | IG_INSTALL_DIR=/usr/local/bin bash
```

### Download binary manually

Grab the latest binary from [Releases](https://github.com/MakFly/instant-grep/releases):

| Platform                | Binary             |
| ----------------------- | ------------------ |
| Linux x86_64            | `ig-linux-x86_64`  |
| macOS x86_64            | `ig-macos-x86_64`  |
| macOS ARM (M1/M2/M3/M4) | `ig-macos-aarch64` |

```bash
# Example: Linux
curl -fsSL https://github.com/MakFly/instant-grep/releases/latest/download/ig-linux-x86_64 -o ~/.local/bin/ig
chmod +x ~/.local/bin/ig
```

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
# Start daemon (keeps index in memory)
ig daemon . &

# Query via Unix socket — 0.2ms response
ig query "pattern" .
```

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

### On-disk format

The index lives in `.ig/` at the project root:

| File           | Format                                            | Size (1,284 files) |
| -------------- | ------------------------------------------------- | ------------------ |
| `metadata.bin` | bincode — file paths, mtimes, git SHA             | 84 KB              |
| `lexicon.bin`  | Hash table: `[NgramKey:u64, offset:u32, len:u32]` | 30 MB              |
| `postings.bin` | Sorted `DocId:u32` arrays, concatenated           | 25 MB              |

The lexicon is memory-mapped (`mmap`). Postings are memory-mapped. Metadata is deserialized via `bincode` (~1ms).

### Default exclusions

38 directories are excluded by default (override with `--no-default-excludes`):

`node_modules` `target` `dist` `build` `.next` `.nuxt` `__pycache__` `.venv` `venv` `vendor` `.git` `.hg` `.svn` `coverage` `.cache` `.turbo` `.output` `.vercel` `tmp` `.temp` `.gradle` `.idea` `.vscode` `.terraform` `.pants.d` `bazel-out` `.mypy_cache` `.ruff_cache` `.pytest_cache` `.tox` `bower_components` `.dart_tool` `.pub-cache` `.cargo` `Pods` and more.

Files larger than 1 MB are also skipped by default (`--max-file-size` to override).

## Benchmarks

Measured with `time` on iautos/apps/web (1,284 source files, Next.js project). Debian, AMD Ryzen, NVMe SSD.

### Wall time (process start to exit)

| Query                             | Candidates | ig (CLI)   | ripgrep |
| --------------------------------- | ---------- | ---------- | ------- |
| `"fetchSellerListings"` (4 hits)  | 4 / 1,284  | **~3ms**   | ~11ms   |
| `"useRouter"` --type ts (74 hits) | 74 / 1,284 | **~3ms**   | ~11ms   |
| `"ZZZZNOTFOUND"` (0 hits)         | 0 / 1,284  | **~3ms**   | ~11ms   |
| Daemon mode (any query)           | —          | **~0.2ms** | N/A     |

> ig wall time is ~3ms regardless of candidate count because the bottleneck is process startup + mmap, not the search itself. The daemon bypasses this entirely.

### ig vs ripgrep

|          | ig                                                            | ripgrep                                      |
| -------- | ------------------------------------------------------------- | -------------------------------------------- |
| Best at  | Projects with persistent index, agent loops, repeated queries | One-off searches, no setup, cold filesystems |
| Weakness | Index build time (~0.2s), larger memory footprint             | Scans all files every time                   |

> **Honest note:** On very large directories (21K+ files), ripgrep with warm disk cache can match ig's speed because its SIMD-optimized scanning is extremely fast. ig's advantage grows with repeated queries and agent usage patterns where the daemon mode shines.

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

### Compared to alternatives

|                      | ig    | ripgrep | Cursor search  | MCP grep server  |
| -------------------- | ----- | ------- | -------------- | ---------------- |
| Index-based          | Yes   | No      | Yes            | No               |
| Latency              | 1.5ms | ~95ms   | ~13ms          | ~95ms + overhead |
| Token cost           | 4K    | 4K      | N/A (internal) | 145K             |
| Works with any agent | Yes   | Yes     | Cursor only    | Needs MCP client |
| Daemon mode          | Yes   | No      | Built-in       | No               |

## Architecture

```
ig
├── index/
│   ├── ngram.rs      — Sparse n-gram extraction (port of danlark1/sparse_ngrams)
│   ├── writer.rs     — Index build: walk → ngrams → hash table + postings
│   ├── reader.rs     — Index query: mmap lexicon + postings
│   ├── postings.rs   — Sorted merge intersection/union
│   └── metadata.rs   — Binary + JSON metadata (bincode)
├── query/
│   ├── extract.rs    — Regex → NgramQuery (via regex-syntax Extractor + covering algo)
│   └── plan.rs       — NgramQuery { And, Or, Ngram, All }
├── search/
│   ├── indexed.rs    — Full pipeline: query → candidates → parallel verify
│   ├── fallback.rs   — Brute-force scan (no index)
│   └── matcher.rs    — File-level regex matching + line extraction
├── daemon.rs         — Unix socket server + client
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
