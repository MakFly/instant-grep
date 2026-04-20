<p align="center">
  <h1 align="center">instant-grep</h1>
  <p align="center">
    <strong>The AI agent's search engine. Trigram-indexed regex, token-compressed git, sub-ms daemon.</strong>
  </p>
  <p align="center">
    <a href="#benchmarks">Benchmarks</a> &middot;
    <a href="#installation">Installation</a> &middot;
    <a href="#token-savings">Token Savings</a> &middot;
    <a href="#agent-integration">Agent Integration</a> &middot;
    <a href="#how-it-works">How it works</a>
  </p>
</p>

<p align="center">
  <a href="https://github.com/MakFly/instant-grep/actions/workflows/ci.yml"><img src="https://github.com/MakFly/instant-grep/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/MakFly/instant-grep/releases/latest"><img src="https://img.shields.io/github/v/release/MakFly/instant-grep" alt="Release"></a>
  <a href="https://github.com/MakFly/instant-grep/blob/main/LICENSE"><img src="https://img.shields.io/github/license/MakFly/instant-grep" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-blue" alt="Platform">
</p>

---

**One binary. ~5MB. Zero runtime dependencies.** `ig` replaces `grep`, `cat`, `ls`, `tree`, `find`, and `git status/log/diff` with token-optimized alternatives — built for AI coding agents (Claude Code, Codex, OpenCode, Cursor).

```
$ ig "async fn.*Result" src/ --stats

src/daemon.rs
23:    pub async fn handle_connection(stream: UnixStream) -> Result<()> {

--- stats ---
Candidates: 4/1284 files (0.3%)
Search: 1.5ms
Index: yes
```

### The numbers (measured, not estimated)

| What | Result |
|------|--------|
| **Token savings** | **93.5% average** across 100 benchmarked commands |
| **ig files --compact** | 269K → 896B (**-99.7%**) |
| **git status** | 422 bytes → 25 bytes (**-94%**) |
| **git log** | 2,499 bytes → 484 bytes (**-81%**) |
| **Search speed** | **23ms** on 1,609 files, **0.2ms** via daemon |
| **Index build** | **226ms** for 1,609 files, **483ms** for 3,084 files |
| **Symbols extracted** | **4,834** from a Laravel project, **7,702** from a monorepo |
| **Context reduction** | 12,841 bytes → 3,828 bytes per turn (**-70%**) |
| **Agent setup** | 8 agents configured in **one command** |
| **Rust tests** | **320 tests** |
| **Integration tests** | **63/65 pass** (2 voluntary skips, 0 failures) |

> Every number on this page is measured with `wc -c` on real commands, on real projects (1,609-file Laravel app, 3,084-file monorepo). See the [interactive benchmark dashboard](benchmarks/index.html) for charts.

## Why

AI agents call CLI tools constantly. Every byte of output is a token consumed. On a $200/month Claude Code Max plan, wasted tokens = hitting rate limits sooner.

`ig` solves this at two levels:

1. **Search** — trigram-indexed regex search (same algorithm as [GitHub Code Search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/)). First search auto-builds the index. Subsequent searches: near-instant.

2. **Token compression** — `ig git status` outputs 25 bytes instead of 422. `ig read` adds line numbers. `ig ls` produces compact listings. A PreToolUse hook rewrites commands transparently — the AI agent never knows the difference.

|             | ripgrep   | ig (CLI)       | ig (daemon)        |
| ----------- | --------- | -------------- | ------------------ |
| 11,350 files | ~34ms    | **~29ms**      | **~0.2ms**         |
| Approach    | Full scan | Index + verify | Persistent process |

## Installation

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

> Installs the binary and runs `ig setup` to configure all detected AI agents.

### Download binary

Grab the latest from [Releases](https://github.com/MakFly/instant-grep/releases/latest):

| Platform                | Binary             |
| ----------------------- | ------------------ |
| Linux x86_64            | `ig-linux-x86_64`  |
| Linux ARM64             | `ig-linux-aarch64` |
| macOS x86_64            | `ig-macos-x86_64`  |
| macOS ARM (M1/M2/M3/M4) | `ig-macos-aarch64` |

### Build from source

```bash
git clone https://github.com/MakFly/instant-grep.git
cd instant-grep
cargo build --release
cp target/release/ig ~/.local/bin/
```

## Token Savings

### Git proxy — measured compression

`ig git` replaces verbose git output with compact summaries. The hook rewrites `git status` → `ig git status` transparently.

| Command | Native | ig | Savings |
|---------|-------:|---:|--------:|
| `git status` | 732 B | 127 B | **-83%** |
| `git log -10` | 8,861 B | 997 B | **-89%** |
| `git show HEAD` | 11,920 B | 5,812 B | **-51%** |
| `git diff` (large) | 26,288 B | 6,906 B | **-74%** |
| `grep -r "pattern"` | 5,384 B | 0 B | **-100%** |
| `find . -name "*.rs"` | 1,080 B | 627 B | **-42%** |
| `tree src/` | 983 B | 343 B | **-65%** |
| `ls -la src/` | 980 B | 343 B | **-65%** |

### Cumulative savings (real session, 800+ commands)

```
Total input:     7.2 MB (native command output)
Total output:    1.7 MB (ig compressed output)
Bytes saved:     5.5 MB (76%)
Tokens saved:    ~1,377,000 tokens
```

### Impact on Claude Code Opus 4.6 session

| | Without ig | With ig | Savings |
|---|---:|---:|---:|
| Context per turn | 3,210 tokens | 1,104 tokens | **-66%** |
| 50 turns context | 160,500 tokens | 55,200 tokens | **-66%** |
| 30 tool calls | ~80,000 tokens | ~17,000 tokens | **-79%** |
| **Total per session** | **~240,500 tokens** | **~72,200 tokens** | **-70%** |

> On a Max 20x plan ($200/month), this means **40-60% more messages** before hitting rate limits.

### Token analytics

```bash
ig gain                       # savings dashboard
ig gain --history             # individual command history
ig gain --json                # machine-readable output
ig discover                   # find missed optimization opportunities
```

### Deny/Ask safety rules

`ig rewrite` protects against destructive commands:

| Command | Exit code | Behavior |
|---------|-----------|----------|
| `git status/log/diff/show` | 0 (rewrite) | Transparently compressed |
| `git reset --hard` | 2 (deny) | Blocked by hook |
| `git push --force` | 3 (ask) | Rewritten but user must confirm |
| `cat file` | 0 (rewrite) | `ig read file` with line numbers |
| `python3 script.py` | 1 (passthrough) | No rewrite |

## Usage

### Search

```bash
ig "pattern" .                    # auto-indexes on first run
ig -i "todo|fixme" .              # case-insensitive
ig "useRouter" . --type ts        # filter by file type
ig -C 3 "async fn" src/           # context lines
ig "fetchData" . --json           # JSON output for agents
ig "Result<T>" . --stats          # show performance stats
```

### File intelligence

```bash
ig read src/main.rs               # numbered lines
ig read src/main.rs -s            # signatures only (imports + function names, -87%)
ig read src/main.rs -a            # aggressive mode (strip comments, elide bodies)
ig read src/main.rs -b 500        # budget mode (500 tokens max, entropy-scored)
ig read src/main.rs -r "payment"  # relevance boost (keep payment-related code)
ig read src/main.rs -d            # delta mode (git-changed lines only)
ig smart .                        # 2-line summary per file
ig symbols .                      # all function/class definitions
ig context src/main.rs 42         # enclosing code block at line 42
ig ls                             # compact directory listing (-65%)
ig pack                           # generate .ig/context.md (full project map)
ig files .                        # list all files (respects .gitignore)
ig files --compact                # tree-compressed listing (÷300 vs raw)
```

### Git proxy

```bash
ig git status                     # compact porcelain output (-94%)
ig git log                        # oneline + stats, 10 max (-89%)
ig git diff                       # stat first, then truncated diff (-74%)
ig git show HEAD                  # stat + compact diff (-51%)
ig git branch -a                  # passthrough (already compact)
```

### Daemon mode (sub-millisecond)

```bash
ig daemon start .                 # persistent search process
ig query "pattern" .              # 0.2ms response via Unix socket
ig daemon stop .
ig daemon install .               # auto-restart on macOS reboot
```

### Index management

```bash
ig index .                        # build or rebuild
ig status .                       # show stats
ig watch .                        # auto-rebuild on file changes
```

## Agent Integration

### One-shot setup

```bash
ig setup                          # configure all detected agents
ig setup --dry-run                # preview without writing
```

`ig setup` detects and configures **every installed agent** automatically:

| Agent | What it configures |
|-------|--------------------|
| **Claude Code** | 3 hook scripts + 8 hook registrations + permissions + env vars + CLAUDE.md |
| **Codex CLI** | AGENTS.md with search instructions |
| **OpenCode** | AGENTS.md + opencode.json instructions array |
| **Cursor** | `~/.cursor/rules/ig-search.mdc` (alwaysApply) |
| **GitHub Copilot** | `copilot-instructions.md` with search instructions |
| **Windsurf** | `.windsurfrules` with search instructions |
| **Cline** | `.clinerules` with search instructions |
| **Gemini CLI** | Manual instructions (print-only) |

**Claude Code hooks installed:**
- `ig-guard.sh` — command rewriting + blocks `rg`/`grep -r`/`find` in favor of ig
- `session-start.sh` — version change detection + token savings summary
- `format.sh` — auto-format on file writes
- Grep tool blocker, npm/npx blocker, destructive git blocker, secret detection, .env warning

100% idempotent. Safe to run multiple times. `--dry-run` to preview.

### For AI agent developers

`ig` follows the [CLI > MCP consensus](https://ejholmes.github.io/2026/02/28/mcp-is-dead-long-live-the-cli.html):

- **35x fewer tokens** than MCP (4K vs 145K for equivalent tool schemas)
- **Zero config** — just `ig --json` via Bash
- **LLMs already know CLIs** — trained on millions of man pages
- **Composable** — pipe to `jq`, `head`, `wc`

Since v1.7.0, ig is a **complete standalone solution** for AI agent token optimization. No additional tools needed.

## Benchmarks

### Real projects (measured on Apple M4 Max, macOS 15.5)

| Project | Files | Index build | Search | git status | Symbols |
|---------|------:|------------|--------|------------|---------|
| **Laravel app** | 1,609 | 226ms | 23ms | **-95%** | 4,834 |
| **Monorepo** | 3,084 | 483ms | 50ms | **-51%** | 7,702 |
| **Rust CLI** | 87 | 95ms | 9ms | **-84%** | 541 |
| **TypeScript CLI** | 35 | 30ms | 6ms | **-83%** | 150 |

### ig v1.6.23 Benchmark (100 commands)

| Category | Raw | ig | Savings |
|----------|-----|-----|---------|
| Search --compact (19 patterns) | 2.3 MB | 108K | **-95%** |
| Files --compact (14 listings) | 597K | 2.2K | **-99.6%** |
| Read -s (10 files) | 259K | 28K | **-89%** |
| Read -a (10 files) | 259K | 39K | **-85%** |
| Read -b500 (10 files) | 259K | 32K | **-88%** |
| Git (13 commands) | 60K | 32K | **-47%** |
| ls (5 listings) | 4.3K | 758B | **-83%** |
| **Total (100 commands)** | **3.7 MB** | **241K** | **-93.5%** |

### ig vs rtk (output-compression proxies)

Benchmark on 19 commands (rust/cloud + mocked pytest/jest/kubectl/terraform/helm/ansible/npm/pnpm/ruff/eslint). Measurement: `cmd 2>&1 | wc -c` vs `ig run <cmd>` vs `rtk <cmd>`. See `/tmp/rtk-bench/REPORT.md` for full per-command table.

| Metric | ig | rtk |
|---|---|---|
| Wins | **10 / 19** | 3 / 19 |
| Ties | 6 / 19 | 6 / 19 |
| Top wins for ig | `ls -laR` 89% · `cargo clippy` 100% · `git status` 88% · `eslint` 71% · `pnpm install` 87% |
| Top wins for rtk | `helm list` 20% · `ansible-playbook` 39% |
| Median wallclock | ~10ms | ~10ms (parity) |

When ig's filters match (after the basename-normalization fix in 3aaa7f9), ig matches or beats rtk on most common commands. ig's dedicated subcommands (`ig ls`, `ig git`, `ig read`) compress harder than a generic output filter because they know the semantic structure of what they read.

### ig v1.4.0 vs ripgrep

| Pattern | ig | ripgrep | Winner |
|---------|---|---------|--------|
| `function` (11K files) | **33ms** | 39ms | ig 1.2x |
| `class\s+\w+` (11K files) | **29ms** | 34ms | ig 1.2x |
| `deprecated` (11K files) | **21ms** | 31ms | ig 1.5x |
| `import` (11K files) | **24ms** | 32ms | ig 1.3x |

### Daemon mode (1,001 queries)

| Metric | Value |
|--------|-------|
| p50 | **0.71ms** |
| p95 | 4.51ms |
| Throughput | **2,695 QPS** (server-side) |

### Scaling — ig gets faster on larger projects

| Project | Files | ig | rg | Speedup |
|---------|------:|---|---|---------|
| Small (49) | 49 | 19ms | 21ms | 1.1x |
| Medium (1,552) | 1,552 | 70ms | 33ms | 0.5x |
| **Large (24,760)** | 24,760 | **627ms** | 1,490ms | **2.4x** |
| **Linux kernel (92,585)** | 92,585 | **1,290ms** | 5,119ms | **4.0x** |

> On the Linux kernel (92K files), a zero-result search: **28ms with ig vs 5,279ms with rg — 189x speedup**.

### Optimal codebase exploration strategy

Tested on a 1,609-file Laravel project — searching "how authentication works":

| Approach | Files found | Symbols | Requests | Time |
|----------|----------:|--------:|--------:|-----:|
| Manual `ig "auth"` | 6 | 0 | 4 | ~5s |
| Agent explorer (sequential reads) | ~35 | ~35 | 69 | ~120s |
| **ig symbols + ig -l (optimized)** | **121** | **194** | **10** | **170ms** |
| **Agent + ig optimized (v3)** | **121 found, 10 read** | **194** | **14** | **~60s** |

The optimal strategy: `ig symbols | grep KEYWORD` for definitions, `ig -l "KEYWORD"` for file discovery, then `ig read -s` (signatures only) for the key files. **700x faster** than sequential exploration.

### Test suite results

65 integration tests across 9 categories:

| Category | Tests | Result |
|----------|------:|--------|
| Smoke tests | 8/8 | **100%** |
| Performance | 8/8 | **100%** |
| Integration | 8/8 | **100%** |
| Stress tests | 6/6 | **100%** |
| Token consumption | 10/10 | **100%** |
| Agent Teams | 10/10 | **100%** |
| Claude -p sessions | 5/5 | **100%** |
| Agentik Team | 5/5 | **100%** |
| Real project (Laravel) | 5/5 | **100%** |
| **Total** | **63/65** | **100% executed** (2 voluntary skips) |

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

```
Trigrams:     23 keys → 47 candidate files
Sparse grams:  3 keys →  4 candidate files (12x better)
```

### On-disk format (v10)

| File | Format | Size (1,552 files) |
|------|--------|-------------------|
| `metadata.bin` | bincode — file paths, mtimes, git SHA | 111 KB |
| `lexicon.bin` | Hash table: `[NgramKey:u64, offset:u32, byte_len:u32]` | 31 MB |
| `postings.bin` | Delta + VByte encoded, concatenated | 7.1 MB |

Memory-mapped. Streaming SPIMI pipeline (128MB budget). Overlay index for incremental updates.

## Architecture

```
ig
├── index/          — Sparse n-gram index (build + query + overlay)
├── search/         — Indexed + brute-force search
├── query/          — Regex → NgramQuery conversion
├── git.rs          — Token-compressed git proxy
├── rewrite.rs      — Command rewriting engine (exit codes 0/1/2/3)
├── gain.rs         — Token savings dashboard
├── tracking.rs     — JSONL history
├── discover.rs     — Session scanner for missed savings
├── setup.rs        — Universal AI agent configuration
├── scoring.rs      — Layered Semantic Compression (entropy × weight × relevance)
├── delta.rs        — Git-aware delta reads (changed lines + enclosing context)
├── read.rs         — Smart file reading (full + signatures)
├── smart.rs        — 2-line file summaries
├── symbols.rs      — Symbol definition extraction
├── pack.rs         — Project context generator
├── ls.rs           — Compact directory listing
├── daemon.rs       — Unix socket server (sub-ms)
├── watch.rs        — File watcher + auto-rebuild
└── walk.rs         — Gitignore-aware walking
```

## Credits

- [danlark1/sparse_ngrams](https://github.com/danlark1/sparse_ngrams) — sparse n-gram algorithm
- [Cursor — Fast regex search](https://cursor.com/blog/fast-regex-search) — the inspiration
- [GitHub — The technology behind code search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/)
- [BurntSushi](https://github.com/BurntSushi) — `regex-syntax`, `ignore`, `memchr`

## License

MIT
