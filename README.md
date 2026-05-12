# instant-grep

**A trigram-indexed `ripgrep` alternative for AI agents — sub-millisecond regex code search in Rust, with 93.5% token savings on `grep` / `cat` / `git` output.**

Drop-in replacement for `grep`, `cat`, `ls`, `find`, `git status/log/diff` — built for Claude Code, Codex, Cursor, OpenCode, Copilot, Windsurf, Cline, and Gemini CLI.

<p align="center">
  <a href="#why-instant-grep">Why ig?</a> &middot;
  <a href="#benchmarks">Benchmarks</a> &middot;
  <a href="#installation">Installation</a> &middot;
  <a href="#token-savings">Token Savings</a> &middot;
  <a href="#agent-integration">Agent Integration</a> &middot;
  <a href="#how-it-works">How it works</a> &middot;
  <a href="#faq">FAQ</a>
</p>

<p align="center">
  <a href="https://github.com/MakFly/instant-grep/actions/workflows/ci.yml"><img src="https://github.com/MakFly/instant-grep/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/MakFly/instant-grep/releases/latest"><img src="https://img.shields.io/github/v/release/MakFly/instant-grep" alt="Release"></a>
  <a href="https://github.com/MakFly/instant-grep/blob/main/LICENSE"><img src="https://img.shields.io/github/license/MakFly/instant-grep" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-blue" alt="Platform: macOS and Linux">
  <img src="https://img.shields.io/badge/rust-2024-orange" alt="Rust 2024 edition">
</p>

---

## TL;DR

- **Trigram-indexed regex** that beats `ripgrep` 2–8× on warm caches with byte-identical match parity.
- **Token-compressed CLI** (`ig git status/log/diff`, `ig read -s`, `ig ls`, …) shipped as drop-ins for AI agents.
- **Single global daemon** — one Unix socket serves every project on the machine, ~6 MB idle, sub-ms hot path.
- **Two-step install**: `curl … install.sh | bash`, then `ig daemon install`. Indexes live in `~/.cache/ig/` (XDG-compliant, no `.ig/` folder to gitignore).

## Why instant-grep?

### vs `ripgrep`

`ripgrep` walks the gitignore tree and opens every candidate file on every query — fast in absolute terms (17–27 ms on a 3 K-file repo), but always proportional to repo size. `ig` builds a sparse trigram index once, then answers in **2.4–8 ms** through a persistent daemon. Median speedup: **2.6×** (measured on a 18 GB monorepo, 5 patterns, hyperfine -N). Match output is **byte-identical** with `rg` — same lines, same counts, same column offsets. The whole point is parity, not approximation.

### vs `rtk` and other agent compressors

`rtk` shells to `ripgrep` on every invocation and post-processes the output. `ig` is the only token compressor that ships its own **persistent index**, which unlocks two things `rtk` cannot replicate without re-implementing one: **`--top N` BM25 ranking** (10/10 byte wins on the 115-case benchmark) and **`--semantic` PMI expansion** (synonyms learned from your own codebase, no ML model). On total bytes + total wall time, `ig` wins both axes simultaneously (896 KB / 1.74 s vs 1.04 MB / 2.88 s).

### For AI agents

Every byte of CLI output is a token consumed. On a $200/month Claude Code Max plan, wasted tokens hit your rate limit sooner. `ig` cuts `git status` by 94 %, `cat large-file.ts` by 96 % (signatures mode), `rg dense-pattern src/` by 60–95 % — measured, not estimated. A PreToolUse hook auto-rewrites `grep` / `rg` / `find` / `cat` / `git` calls so the agent never knows the difference. **Zero-config** via `ig setup`: 8 agents configured in one command.

```
ig ───── ~/.cache/ig/  (one daemon, one socket, all your projects)
 │             │
 │             ├── daemon/
 │             │     ├── daemon.sock         ← single multi-tenant socket
 │             │     ├── daemon.pid
 │             │     └── daemon.log[.1…5]    ← rotated at 5 MB
 │             │
 │             ├── projects/
 │             │     ├── <hash-of-rootA>/    ← lexicon.bin, postings.bin, seal, …
 │             │     ├── <hash-of-rootB>/
 │             │     └── ...
 │             │
 │             ├── by-name/                  ← human-friendly symlinks
 │             │     ├── tilvest-app -> ../projects/2e0c…
 │             │     └── ...
 │             │
 │             ├── tee/                      ← centralized tee output
 │             └── manifest.json             ← global registry (cheap cache-ls)
 ├── search / git proxy / ls / read / pack
 └── gc / migrate / cache-ls
```

## Release highlights

- **Indexes live in the XDG cache** (`~/.cache/ig/`) since v1.15.0 — your projects stay clean (no `.ig/` folder to gitignore). `find_root` also recognises `package.json`, `Cargo.toml`, `go.mod`, etc., so non-versioned projects no longer scatter stray indexes.
- **One global daemon** since v1.16.0 — multi-tenant, single Unix socket, single systemd-user / launchd unit. Replaces the previous one-daemon-per-project design. **~14× less RAM** in real-world use (5–20 MB total instead of 60 MB × N).
- **Precision search (v1.17.0+)** — vbyte posting codec + masked n-grams (bloom / loc / zone). Sub-byte filtering before any `read(2)`. `INDEX_VERSION 13`.
- **Push reload via FSEvents (v1.18.0)** — `.ig/seal` 16-byte atomic publish marker + `notify` watcher on `.ig/`. Out-of-band `ig index .` from another shell propagates to the daemon **without waiting for the next query**. Pull (per-query 16-byte read) stays authoritative as the safety net. See [`docs/specs/SPEC-daemon-cache-invalidation.md`](docs/specs/SPEC-daemon-cache-invalidation.md) for the full design.
- **Navigable cache layout (v1.19.0)** — the cache root is now organized into `daemon/` (sock + pid + rotated logs), `projects/<hash>/`, `by-name/<slug>` symlinks for human inspection, `tee/`, and `manifest.json`. Migration from the pre-v1.19 flat layout is automatic and idempotent on first launch.
- **Self-healing setup (v1.19.1+)** — `ig setup` writes a managed block (`<!-- IG-MANAGED-BLOCK -->`) into agent rule files (`CLAUDE.md`, `AGENTS.md`, `~/.claude/rules/tools/ig.md`). On binary upgrades, `ig update` re-runs `ig setup --quiet` so the block always reflects the current contract; only drifted entries surface, no wall of "already up-to-date".

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
| **ig vs ripgrep 14.1.1 — wall time** (v1.11.0, 5 patterns on iautos/apps 18 GB) | **2.2× to 7.8× faster** (median 2.6× faster) |
| **ig vs ripgrep — match parity** | **5/5 patterns identical** (file count + total matches byte-for-byte) |
| **Daemon auto-route gain** (v1.11.0, 10-query burst) | **-15% wall, -70% user CPU** vs in-process search |
| **Single-query latency** (warm, daemon, iautos/apps) | **2.4–8.1 ms** depending on pattern |
| **ig vs rtk total bytes** (v1.10.0, 115 cases on a 347K-file monorepo) | **896 KB vs 1.04 MB** (ig wins) |
| **ig vs rtk total time** (same 115 cases) | **1.74 s vs 2.88 s** (ig 40% faster) |
| **BM25 `--top N` vs rtk** | **10/10 bytes wins**, 7/10 time wins (rtk has no index) |
| **`--semantic` PMI vs rtk** | **5/5 bytes wins** — synonyms learned from your repo |
| **Token savings** | **93.5% average** across 100 benchmarked commands |
| **ig files --compact** | 176K → 149B (**-99.9%**) on a 3K-file project |
| **git status** | 422 bytes → 25 bytes (**-94%**) |
| **git log** | 2,499 bytes → 484 bytes (**-81%**) |
| **Index build** | **226ms** for 1,609 files, **483ms** for 3,084 files |
| **Symbols extracted** | **4,834** from a Laravel project, **7,702** from a monorepo |
| **Context reduction** | 12,841 bytes → 3,828 bytes per turn (**-70%**) |
| **Agent setup** | 8 agents configured in **one command** |
| **Rust tests** | **438 tests** (389 bin + 49 goldens) |
| **Integration tests** | **63/65 pass** (2 voluntary skips, 0 failures) |
| **Commands rewritten** | **91 bins** across 42 TOML filters (v1.9.0) |

### ig vs ripgrep 14.1.1 (v1.11.0, iautos/apps 18 GB, warm cache, hyperfine -N)

| pattern             | ig (daemon) | rg 14.1.1 | ig faster |
| ------------------- | ----------: | --------: | --------: |
| `useEffect`         | 5.9 ms      | 18.3 ms   | **3.1×**  |
| `createServer`      | 2.4 ms      | 18.8 ms   | **7.8×**  |
| `fn\s+\w+_test`     | 3.5 ms      | 27.4 ms   | **7.8×**  |
| `async function`    | 8.1 ms      | 18.2 ms   | **2.2×**  |
| `export default`    | 6.9 ms      | 18.0 ms   | **2.6×**  |

`rg` spends ~17–27 ms walking the gitignore tree and opening 3 000 candidate files. `ig`'s trigram filter cuts that to ~50–200 candidates *before* any file is touched — `User: 1.5 ms, System: 1.5 ms` average. Match counts identical on every pattern (no false positives, no missed lines).

> Every number on this page is measured with `wc -c` / `hyperfine` on real commands, on real projects (1,609-file Laravel app, 3,084-file monorepo, 347K-file iautos SaaS). See the [v1.10.0 benchmark artefacts](documentation/public/bench/v1.10.0/) for the older CSV + per-domain tables.

## Two-level optimisation

`ig` attacks token waste at two layers simultaneously:

1. **Search** — trigram-indexed regex search (same algorithm class as [GitHub Code Search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/)). First search auto-builds the index. Subsequent searches: near-instant.

2. **Token compression** — `ig git status` outputs 25 bytes instead of 422. `ig read --plain` is byte-exact with `cat`, or `-s` gives signatures-only (−95% on large code files). `ig ls` produces compact listings. Compact search mode (`IG_COMPACT=1`) caps matches + truncates long lines for −60 to −95% on `grep`/`rg`. A PreToolUse hook rewrites commands transparently — the AI agent never knows the difference.

|              | ripgrep 14.1.1 | ig (CLI, in-proc) | ig (daemon, auto-route) |
| ------------ | -------------- | ----------------- | ----------------------- |
| iautos/apps (3K files, 18 GB) | ~18–27 ms | ~3–7 ms | **~2.4–8 ms** (auto-spawned, transparent) |
| Approach     | Full scan      | Index + verify    | Persistent hot process  |

## Installation

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

> Installs the binary and runs `ig setup` to configure all detected AI agents.

### Download binaries

Since **v1.13.0**, `ig` ships as two artefacts per platform — a tiny C shim (in your `PATH`) and a hidden Rust backend. Grab both from [Releases](https://github.com/MakFly/instant-grep/releases/latest):

| Platform                | Shim (→ `~/.local/bin/ig`)  | Backend (→ `~/.local/share/ig/bin/ig-rust`) |
| ----------------------- | --------------------------- | ------------------------------------------- |
| Linux x86_64            | `ig-linux-x86_64`           | `ig-linux-x86_64-rust`                      |
| Linux ARM64             | `ig-linux-aarch64`          | `ig-linux-aarch64-rust`                     |
| macOS x86_64            | `ig-macos-x86_64`           | `ig-macos-x86_64-rust`                      |
| macOS ARM (M1/M2/M3/M4) | `ig-macos-aarch64`          | `ig-macos-aarch64-rust`                     |

The shim resolves the backend through `$IG_BACKEND` → `~/.local/share/ig/bin/ig-rust` → `/usr/local/share/ig/bin/ig-rust` → first `ig-rust` on `PATH`. Use `install.sh` to do this layout automatically (recommended).

### Build from source

```bash
git clone https://github.com/MakFly/instant-grep.git
cd instant-grep
cargo build --release
cp target/release/ig ~/.local/bin/
```

## Token Savings

### v1.8.2 benchmarks — measured on real projects

Numbers below come from a monorepo (Next.js frontend ~12 MB + Symfony/PHP backend ~5.5 MB). Every row is a single `wc -c` comparison between the raw command and its ig-rewritten equivalent.

| Category | Command | Raw | ig | Savings |
|---|---|---:|---:|---:|
| **ls** | `ls -la` | 3,086 B | 577 B | **−81%** |
| **ls** | `ls -laR app/` | 81,866 B | 232 B | **−99.7%** *(ig ls is a flat tree — not 1:1 with `ls -laR`)* |
| **cat** small | `cat package.json` | 5,187 B | 5,187 B | 0% *(parity — no regression)* |
| **cat** large code | `cat ApiExceptionSubscriber.php` → `-s` | 10,929 B | 2,138 B | **−80%** |
| **cat** large code | `cat market-insights-actions.ts` → `-s` | 8,773 B | 339 B | **−96%** |
| **grep/rg** dense | `rg 'public function' src/` (PHP) | 243,740 B | 14,360 B | **−94%** |
| **grep/rg** dense | `rg 'useState' features/ app/` | 95,021 B | 15,934 B | **−83%** |
| **grep/rg** dense | `rg 'Entity' src/` (PHP) | 122,345 B | 11,758 B | **−90%** |
| **grep/rg** medium | `rg 'export function' app/` | 57,812 B | 21,809 B | **−62%** |
| **grep/rg** sparse | `rg 'fn build' src/` (10 matches) | 674 B | 642 B | −5% *(physical floor)* |
| **git** | `git status` | 732 B | 127 B | **−83%** |
| **git** | `git log -10` | 8,861 B | 997 B | **−89%** |
| **git** | `git diff` (large) | 26,288 B | 6,906 B | **−74%** |
| **docker** | `docker ps` | 1,792 B | 593 B | **−67%** |
| **docker** | `docker compose ps` | 1,792 B | 593 B | **−67%** |
| **docker** | `docker logs` | 1,909 B | 886 B | **−54%** |
| **JS/TS** | `jest --verbose` | 6,125 B | 910 B | **−85%** |
| **JS/TS** | `bun test` | 3,467 B | 301 B | **−91%** |
| **JS/TS** | `playwright test` | 3,984 B | 688 B | **−83%** |
| **PHP** | `phpunit` | 1,340 B | 698 B | **−48%** |
| **PHP** | `pest` | 1,220 B | 651 B | **−47%** |

> **How grep/rg compaction works** (`IG_COMPACT=1`, auto-set by rewrites): line truncation at 100 chars (UTF-8 safe), per-file cap of 10 matches, global cap of 200 matches with an explicit `… global cap reached` marker. Inter-file blank lines and `--` separators are dropped. Matches rtk's `--context-only` gains on dense patterns, beats rtk on sparse ones (no header overhead).

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

### Command rewriting — full RTK parity (v1.9.0)

`ig rewrite` now matches [`rtk rewrite`](https://github.com/rtk-ai/rtk) on every depth feature and exceeds it on breadth. The hook (`~/.claude/hooks/ig-guard.sh`) is a thin shell delegator — all intelligence lives in the Rust binary. Measured in a 4-round × 30-session `claude -p` benchmark, 28 / 28 piped `rg`/`grep -r`/`find -name` commands are now silently rewritten (0 `BLOCK` errors visible to the model).

| Feature | `ig rewrite` | `rtk rewrite` |
|---|:---:|:---:|
| Thin shell hook (stdin JSON delegator) | ✅ | ✅ |
| Pipelines (`rg pat src \| head -20`) | ✅ | ✅ |
| Compounds (`cargo test && ls -la`) | ✅ | ✅ |
| ENV prefix (`RUST_LOG=debug rg …`) | ✅ | ✅ |
| `sudo` / `env` wrappers | ✅ | ✅ |
| Absolute binary paths (`/usr/bin/grep`) | ✅ | ✅ |
| Git global options (`git -C path log`) | ✅ | ✅ |
| Deny rules (`rm -rf /`, `git reset --hard`) | ✅ | ✅ |
| Ask rules (`git push --force`) | ✅ | ✅ |
| Dedup consecutive identical output lines | ✅ | ✅ |
| Rewritten command categories | **91** | 72 |

All features are quote-aware: `|`/`;`/`&&` inside `"…"` or `'…'` are preserved literally.

**Example rewrites** (what the agent typed → what actually runs):

```
grep -r "fn main" src --include="*.rs" | wc -l
  → IG_COMPACT=1 ig "fn main" src | wc -l

RUST_LOG=debug rg useState features/
  → RUST_LOG=debug IG_COMPACT=1 ig "useState" features/

/usr/bin/grep -rn pattern .
  → IG_COMPACT=1 ig "pattern"

git -C /tmp/repo log --oneline
  → ig git log --oneline

find src -type f -name "*.rs"
  → ig files --glob "*.rs"

cargo test && ls -la src
  → ig run cargo test && ig ls src
```

### Deny/Ask safety rules

`ig rewrite` protects against destructive commands:

| Command | Exit code | Behavior |
|---------|-----------|----------|
| `git status/log/diff/show` | 0 (rewrite) | Transparently compressed |
| `git reset --hard` | 2 (deny) | Blocked by hook |
| `git push --force` | 3 (ask) | Rewritten but user must confirm |
| `cat file` | 0 (rewrite) | `ig read --plain file` (byte-exact) or `-s` on large code files |
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
ig --top 10 "pattern" .           # BM25 ranking, keep top 10 by relevance (v1.10.0)
ig --semantic "error" .           # expand query with PMI-learned synonyms (v1.10.0)
```

### Compact search mode (v1.8.2+)

Set `IG_COMPACT=1` (or let the PreToolUse hook do it when rewriting `grep`/`rg`) to enable aggressive output compaction:

```bash
IG_COMPACT=1 ig "pattern" src/    # capped, truncated, no separators
```

What changes:
- Line truncation at **100 chars** (UTF-8 safe, `…` marker, match stays visible)
- Per-file cap: **10 matches** with `… +N more` footer
- Global cap: **200 matches** with `… global cap reached` marker
- Inter-file blank line + `--` separators between non-contiguous matches are dropped

Override individual caps:
```bash
IG_LINE_MAX=80 IG_MAX_MATCHES_PER_FILE=5 IG_MAX_MATCHES_TOTAL=100 ig "pattern" src/
```

Typical gains on real projects: **−60 to −94%** on dense patterns (`rg 'public function' src/` on a Symfony codebase: 244 KB → 14 KB).

### BM25 ranking — `--top N` (v1.10.0)

Regex search returns every match in filesystem order. That's fine for a human skimming 20 hits — it's wasteful when there are 2 000 of them and only 5 actually matter. `--top N` scores each matched file with a textbook Okapi BM25 and keeps only the N highest-ranked:

```bash
$ ig --top 5 useState
apps/.../create-conversational/vehicle-edit-step-dialog.tsx
  3: import { useState, useMemo } from "react";
 73:   const [value, setValue] = useState(formData.saleMode);
107:   const [brandId, setBrandId] = useState(formData.brand);
…
```

Score = `idf · (tf · (k1 + 1)) / (tf + k1 · (1 − b + b · dl / avdl))` with `k1 = 1.5`, `b = 0.75`. `tf` is the match count per file, `dl` is the file byte-size, `avdl` is the mean across matches. Dense hits in short files rank first — the files where the concept is actually implemented, not the files that happen to mention it in a comment.

On iautos: `ig --top 10 "export default"` returns **743 bytes** of curated hits; `rtk grep "export default"` returns a flat-compressed 19 KB dump. Not better compression — *better content*, because rtk has no index and cannot rank.

### Semantic query expansion — `--semantic` (v1.10.0)

Statistical synonym expansion, **no ML model, no download**. During `ig index`, a second pass tokenises every line, counts co-occurrences in a 5-line sliding window, and persists a PMI-ranked top-10 neighbour table to `.ig/cooccurrence.bin`. At query time, `ig --semantic error` rewrites the regex to `\b(error|catch|throw|exception|…)\b` and lets the trigram pre-filter do the heavy lifting:

```bash
$ ig --semantic --top 5 throw
(semantic: expanded 'throw' → got, inattendu, denied, autorisé, trouvée, manquant)
apps/packages/reader-api-vo/scripts/test-rest-e2e.ts
 44:   throw new Error("HTTP server did not become ready in time");
…
```

The synonyms are **learned from your own codebase** — if you have `VehicleWantedError` or `iautosPaymentException`, they'll show up in the neighbour tables alongside the common vocabulary. Levy & Goldberg ([NeurIPS 2014](https://papers.nips.cc/paper_files/paper/2014/file/feab05aa91085b7a8012516bc3533958-Paper.pdf)) proved skip-gram word2vec with negative sampling implicitly factorises the shifted-PMI matrix, so direct PMI recovers most of the neighbourhood quality of a learned embedding at a fraction of the cost.

Controls:
- Disable build entirely: `IG_SEMANTIC=0 ig index`
- Daemon warms skip semantic indexing by default to keep background RSS bounded; set `daemon_semantic_index = true` or `IG_SEMANTIC=1` to opt in.
- Opt-out per query: just don't pass `--semantic`
- Inspect: the stderr line `(semantic: expanded 'x' → …)` shows exactly what was added — no magic

Expansion quality depends on how often a term co-occurs with others in your corpus: `throw`, `payment`, `auth`, `config` work well on iautos; rare terms may get a weak expansion or none at all.

### File intelligence

```bash
ig read src/main.rs               # numbered lines
ig read src/main.rs --plain       # no line numbers, byte-exact with `cat` (v1.8.2+)
ig read src/main.rs -s            # signatures only (imports + function names, -95% on large code)
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

### Daemon mode (sub-millisecond, multi-tenant since v1.16.0)

A single global daemon serves searches for **every** indexed project on the
machine. One process, one socket, one systemd / launchd unit. Tenants are
opened lazily on first query and kept in an LRU cache (default cap 8, set
`IG_DAEMON_TENANTS_MAX` or `IG_DAEMON_MAX_ACTIVE_PROJECTS` to override).

```bash
ig daemon start                   # start the global daemon (foreground or backgrounded)
ig daemon status                  # PID + socket + RSS / RAM limits
ig daemon stop                    # SIGTERM the daemon
ig daemon install                 # systemd-user (Linux) or launchd (macOS), auto-start on login

ig query "pattern" /path/to/proj  # 0.06–1.3 ms response via the global Unix socket
```

**RAM governor**: the daemon has soft/hard RSS caps. Defaults are soft 768 MB,
hard 1 GB, with a 60 s cooldown after a hard stop so Claude/Codex hooks cannot
relaunch it in a loop. Under soft pressure the daemon evicts tenant caches and
inactive watchers, then pauses new background warms/rebuilds until memory drops.

```toml
# ~/.config/ig/config.toml
[limits]
daemon_soft_rss_mb = 768
daemon_hard_rss_mb = 1024
daemon_cooldown_secs = 60
daemon_max_active_projects = 8
daemon_project_idle_secs = 300
index_memory_mb = 64
index_batch_size = 250
semantic_index = true
daemon_semantic_index = false
```

Equivalent env overrides: `IG_DAEMON_SOFT_RSS_MB`, `IG_DAEMON_HARD_RSS_MB`,
`IG_DAEMON_COOLDOWN_SECS`, `IG_INDEX_MEMORY_MB`, `IG_INDEX_BATCH_SIZE`,
`IG_SEMANTIC`, `IG_DAEMON_TENANTS_MAX`.

**Wire format**: each query carries the project root in its JSON payload
(`{"root": "/abs/path", "pattern": "...", …}`), so the daemon dispatches
internally without needing per-project sockets.

**Boot-time cleanup**: when v1.16.0+ starts, it SIGTERMs any leftover
per-project daemons, removes their `/tmp/ig-*.sock` files, and takes over.
Idempotent.

#### Watcher and warm projects (since v1.17.0)

```bash
ig warm                           # warm the current project (idempotent)
ig projects list                  # active projects + idle seconds
ig projects forget /path/to/proj  # drop from the active set, free its watcher
```

Each warmed project gets its own `notify` watcher. When source files change,
the daemon rebuilds the overlay in the background and reloads its
`IndexReader`. Shell hooks (`zsh` / `bash` / `fish`) and the session-start
hook now use `ig warm` instead of `ig daemon start`; the managed shell-hook
block is self-updating across versions (re-run `ig setup` to rewrite in
place).

#### Cache invalidation — push + pull (since v1.18.0)

The daemon learns about every rebuild via two complementary channels:

- **Pull (authoritative)** — every query reads `.ig/seal` (a 16-byte file:
  `[u64 generation, u64 finalized_at_nanos]`) and reloads if it advanced.
  The seal is atomically renamed as the **final act** of every rebuild, so
  observing generation N guarantees all artifacts of generation N are
  already published.
- **Push (best-effort)** — a second `notify` watcher on `.ig/` fires
  `reload_tenant_if_open` on `seal` / `seal.tmp` events. An out-of-band
  `ig index .` from another shell propagates to the daemon in ~10 ms,
  without waiting for the next query.

Push optimises steady-state; pull closes the loop if FSEvents misses an
event (which `notify` is known to do under load on macOS, on NFS, or on
some Docker bind-mount configurations). Full contract and invariants:
[`docs/specs/SPEC-daemon-cache-invalidation.md`](docs/specs/SPEC-daemon-cache-invalidation.md).

### Cache management (since v1.15.0)

Indexes live in `~/.cache/ig/projects/<hash-of-root>/` (XDG-compliant).
Set `IG_LOCAL_INDEX=1` to fall back to `<root>/.ig/` for a project, or
`IG_CACHE_DIR=/path` to relocate the whole cache.

```bash
ig cache-ls                       # list every cached project (size, last_used)
ig migrate [--dry-run]            # move <root>/.ig/ to the XDG cache
ig gc [--days N] [--dry-run]      # drop entries whose root is gone, or unused for N days
ig gc --max-size 5GB --dry-run    # preview LRU pruning when the cache exceeds a cap
```

Cache GC also runs automatically on `ig` startup, at most once per hour by
default. It removes orphaned projects, drops entries unused for 30 days, and
keeps total cache size under 5 GB by pruning least-recently-used entries. Tune
with `[cache]` in `~/.config/ig/config.toml` or env vars:
`IG_AUTO_GC`, `IG_CACHE_GC_INTERVAL_SECS`, `IG_CACHE_GC_DAYS`,
`IG_CACHE_MAX_SIZE_MB`.

**Layout (v1.19.0+)** — the cache root is now grouped by purpose. `ensure_layout()`
runs at the entry of every command and migrates pre-v1.19 installs (hash dirs at
the root, `daemon.{sock,pid,log}` mixed in) to the new structure. Idempotent and
safe under concurrent invocations via a create-only `.layout.lock` file. Browsing
the cache is now meaningful:

```bash
ls ~/.cache/ig/by-name/           # symlinks: <project-name> → ../projects/<hash>
cat ~/.cache/ig/manifest.json     # registry: hash, root, size, last_used per entry
ls ~/.cache/ig/daemon/            # daemon.sock, daemon.pid, daemon.log
```

If you upgrade across the v1.19 boundary, the migration is invisible: any
pre-v1.19 daemon still running gets `SIGTERM`ed (PID file at the legacy path),
old hash dirs move into `projects/`, daemon files into `daemon/`, and a v1.19
daemon takes over on the new socket. Duplicate entries (rare; can happen if
a stale daemon recreated a hash dir mid-migration) are resolved by `mtime`
— newest wins.

**Project root detection** (`find_root`) recognises both `.git/` and project
markers (`package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`,
`deno.json`, `composer.json`, `bun.lock`, …). Searches from any subdirectory
of a Next.js / Cargo / Go monorepo resolve to the same root → one shared
index, no duplicates.

### Index management

```bash
ig index .                        # build or rebuild
ig status .                       # show stats
ig watch .                        # auto-rebuild on file changes
```

### Update management

`ig update` has three jobs:

1. update the `ig` binary itself from the latest GitHub release;
2. re-run `ig setup --quiet` so `~/.claude/` hooks/rules and `~/.codex/AGENTS.md`
   stay aligned with the current binary;
3. refresh project indexes when the index format changed or new projects need
   an index.

By default, `ig update` updates the binary and syncs agent config:

```bash
ig update                         # update ig + sync Claude/Codex agent config
ig update --self-only             # skip index refresh, still sync agent config
```

Refresh indexes without touching the binary:

```bash
ig update --indexes               # refresh the current project index
ig update .                       # same: path implies index refresh
ig update /path/to/project        # refresh one explicit project
```

Detect projects under a directory and refresh all of them:

```bash
ig update --all ~/Documents/lab/sandbox
ig update --all ~/work
```

`--all` detects project roots by looking for markers such as `.git`,
`package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`, `composer.json`,
`pnpm-workspace.yaml`, `bun.lock`, `Gemfile`, `pom.xml`, and similar files.
It then builds missing indexes and rebuilds stale indexes whose metadata does
not match the current `INDEX_VERSION`.

Safety rules for broad scans:

- hidden directories under the scanned root are skipped (`.bun`, `.cache`,
  `.cargo`, `.nvm`, `.oh-my-zsh`, `.vscode-server`, etc.);
- dependency/build folders are skipped (`node_modules`, `target`, `vendor`,
  `dist`, `build`, `.next`, `.turbo`, coverage folders, virtualenvs, etc.);
- XDG cache entries pointing to those skipped locations are ignored;
- if you explicitly pass a hidden directory as the root, `ig` treats that as an
  intentional target.

For remote machines, install/update the binary once, then refresh indexes with
one command:

```bash
ssh kev@192.168.1.57 '~/.local/bin/ig update --all ~/Documents/lab/sandbox'
```

Scanning the whole home is allowed, but usually less useful than targeting the
workspace folder:

```bash
ssh kev@192.168.1.57 '~/.local/bin/ig update --all ~'
```

Manual alternatives remain available when you want tighter control:

```bash
ig status /path/to/project        # check whether an index exists/stale
ig index /path/to/project         # force a full rebuild for one project
ig watch /path/to/project         # keep one project rebuilt on file changes
ig cache-ls                       # inspect cached indexes
ig gc --dry-run                   # preview orphan/stale cache cleanup
ig gc --max-size 5GB --dry-run    # preview size-cap LRU cleanup
ig gc                             # remove orphan cache entries
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

### ig vs rtk — full benchmark (v1.10.0)

**115 cases across 12 domains**, run on the `iautos` SaaS monorepo (347 843 files raw, 3 146 after ig's default excludes). Methodology: 2 warm-up passes + **median of 3** wall-time runs per case. Bytes are deterministic (one measurement). Full raw data in `documentation/public/bench/v1.10.0/`.

| Headline | ig 1.10.0 | rtk 0.37.2 |
|---|---:|---:|
| **Total bytes emitted** | **896 KB** | 1.04 MB |
| **Total wall time** | **1.74 s** | 2.88 s |
| **Bytes wins** | **57 / 115** | 54 / 115 *(tie 4)* |
| **Time wins** | **80 / 115** | 27 / 115 *(tie 8)* |

**ig wins on aggregate bytes and wall time simultaneously for the first time.**

#### Per-domain breakdown

| # | Domain | ig bytes wins | rtk bytes wins | ig time wins | rtk time wins |
|---|---|---:|---:|---:|---:|
| 1 | literal search | 5 | 5 | **9** | 1 |
| 2 | regex search | 3 | **7** | **6** | 4 |
| 3 | flag variants | **7** | 3 | **9** | 1 |
| 4 | listing | 2 | **8** | **7** | 3 |
| 5 | read full | 0 | **10** | **8** | 0 |
| 6 | read signatures | **9** | 1 | **10** | 0 |
| 7 | git proxy | **7** | 2 | **8** | 0 |
| 8 | varied identifiers | 3 | 4 | **10** | 0 |
| 9 | smart summaries | 4 | **6** | 0 | **10** |
| 10 | generic proxy | 2 | **8** | 4 | 3 |
| 11 | **`--top` BM25** | **10** | 0 | **7** | 3 |
| 12 | **`--semantic` PMI** | **5** | 0 | 2 | 2 |

#### Where rtk still wins — and why it's a trade-off, not a bug

- **Read full (10/10 bytes to rtk)** — ig keeps the `   42: content` line-number prefix because it's what lets the Edit tool round-trip precisely. Dropping it saves ~15 % bytes per file and halves the utility. Deliberate.
- **Listing / smart dir singles** — rtk's `rtk ls` emits a placeholder 8 B for top-level dirs. Fewer bytes, less information; we emit a compact listing that's still actionable.

#### Where ig is categorically ahead — rtk cannot match without a persistent index

- **`--top N` BM25 ranking** — 10 / 10 bytes wins. Example: `ig --top 10 "export default"` = 743 B; `rtk grep "export default"` = 19 403 B — same query, **−96 %**. rtk has no `tf` / `df` / `avdl` so it cannot rank; it can only flat-compress.
- **`--semantic` PMI expansion** — 5 / 5 bytes wins. Example: `ig --semantic --top 5 throw` = 3 368 B with synonyms learned from the repo; `rtk grep throw` = 17 717 B of literal matches. Building a cooccurrence matrix would require rtk to ship its own index layer.
- **Sub-ms daemon** — not in this run, but `ig daemon` serves queries at p50 = 0.7 ms through a Unix socket; rtk shells to ripgrep on every invocation.

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

### Distribution: C shim + hidden Rust backend (v1.13.0)

```
┌────────────────────────┐
│ ~/.local/bin/ig        │   35 KB C shim, in $PATH
│ (C shim, in PATH)      │
└───────────┬────────────┘
            │ hot path: argv → daemon socket (no execve)
            │ cold path: execve($IG_BACKEND or fallback)
            ▼
┌──────────────────────────────────────┐
│ ~/.local/share/ig/bin/ig-rust        │   5.1 MB Rust backend, hors $PATH
│ (Rust backend)                       │
└──────────────────────────────────────┘
```

A single `ig` name in your `PATH`. The shim handles the hot subcommands (`search`, `grep`, `files`, `count`) entirely in C — argv parse, root resolve, daemon socket round-trip — for sub-2 ms cold start. Cold-path subcommands (`index`, `setup`, `update`, …) `execve` the backend. Backend resolution: `$IG_BACKEND` → user share → system share → first `ig-rust` on `PATH`.

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

### On-disk format (v13)

| File | Format | Size (1,552 files) |
|------|--------|-------------------|
| `metadata.bin` | bincode — file paths, mtimes, git SHA | 111 KB |
| `lexicon.bin` | Hash table: `[NgramKey:u64, offset:u32, byte_len:u32, bloom:u8, loc_mask:u8]` | 31 MB |
| `postings.bin` | Tagged VByte `PostingEntry { doc_id, next_mask, loc_mask, zone_mask }` streams; dense lists include skip blocks | 7.1 MB |

Memory-mapped. Streaming SPIMI pipeline (64 MB default budget, configurable via `index_memory_mb` / `IG_INDEX_MEMORY_MB`). Overlay index for incremental updates. Query execution uses per-document bloom, loc and exact small-position masks as Cursor-style prefilters; v13 skip blocks carry aggregate masks so selective intersections can jump through dense posting lists before final regex verification.

### BM25 ranking (v1.10.0)

When `--top N` is set, the candidate file list from the trigram intersection is scored with Okapi BM25:

```
score(file) = idf · (tf · (k1 + 1)) / (tf + k1 · (1 − b + b · dl / avdl))
            k1 = 1.5, b = 0.75
            tf = match count in the file
            dl = file byte size
            avdl = mean dl across the result set
```

Scoring happens *after* the regex verification pass (so only real matches are considered) and adds one `stat(2)` per candidate. On the 115-case bench, `--top N` never takes more than 50 ms end-to-end, even for patterns that match in 300+ files.

### Semantic layer — PMI, no ML model (v1.10.0)

`--semantic` piggy-backs on a second index built alongside the trigram one:

```
ig index  ─┬─▶ trigram + filedata + symbols       (existing)
            └─▶ cooccurrence.bin                    (new)

ig --semantic <word> ─▶ lookup top-6 PMI neighbours
                     ─▶ build regex \b(word|n1|…|n6)\b
                     ─▶ normal trigram+regex search
                     ─▶ optional BM25 rerank via --top
```

During index build, every line is tokenised (camelCase / snake_case / acronym-aware), and co-occurrences in a 5-line sliding window are counted. At finalisation we compute count-weighted PPMI per pair:

```
PMI(a, b)     = log( p(a, b) / (p(a) · p(b)) )
score(a, b)   = PMI · log(count + 1)     (rejects rare-word coincidences)
```

…and persist the top-10 neighbours per token to `.ig/cooccurrence.bin` (bincode, ~1.5 MB on a 3K-file repo). Thresholds `MIN_PAIR_COUNT = 15` and `MIN_TOKEN_COUNT = 10` kill PMI's well-known low-frequency bias.

Theoretical basis: Levy & Goldberg, [*Neural Word Embedding as Implicit Matrix Factorization*](https://papers.nips.cc/paper_files/paper/2014/file/feab05aa91085b7a8012516bc3533958-Paper.pdf) (NeurIPS 2014) — direct PMI is *the* objective that skip-gram word2vec with negative sampling implicitly optimises.

### OpenAI embeddings — opt-in POC (v1.14.0)

PMI gives you semantic *expansion* (synonyms learned from your repo) on top of literal matching. For natural-language queries that don't share any token with the target code (`"function that cancels a Stripe subscription"` → `unsubscribe()`), you need **dense embeddings**. v1.14.0 ships a pedagogical POC, **disabled by default at two layers**:

| Layer | Mechanism | Controls |
|---|---|---|
| **Compile-time** | `cargo build --features embed-poc` | Whether the `embed-poc` subcommand exists in the binary at all (default: absent). |
| **Runtime** (v1.14.2) | `ig emb on / ig emb off` | Whether it executes when present (default: off). |

```bash
# 1. Compile-time opt-in
cargo build --release --features embed-poc      # subcommand now compiled in
ig embed-poc --help                              # visible

# 2. Runtime toggle (lives in ~/.config/ig/embed.toml)
ig emb status                                    # disabled (default)
ig emb on                                        # enabled
ig emb off                                       # back to disabled

# 3. Try to use embed-poc while runtime is OFF → friendly refusal
$ ig embed-poc hello "test"
Error: embeddings are disabled.
Enable with:  ig emb on
```

The POC is intentionally tiny — JSON store, brute-force cosine, 40-line chunker — so the math is readable. **The shipped binary contains zero OpenAI client code** unless you opt in at compile-time. Even after that, the runtime toggle defaults to off so no network call ever fires by accident. Users without an API key fall back to the regular trigram path (`ig search "pattern"`) which is sub-ms, no network, no cost.

```
ig embed-poc index ./src
   │
   ├─▶ chunk files (40 lines, 5 overlap)             ──▶ 768 chunks
   ├─▶ batch-embed via OpenAI text-embedding-3-small  ──▶ 1536 floats / chunk
   └─▶ persist to .ig/poc-embeddings.json             ──▶ ~30 MB on a 3 k-file repo

ig embed-poc search "function that cancels a Stripe subscription"
   │
   ├─▶ embed the query (1 OpenAI call, ~$0.0000002)
   ├─▶ rayon par_iter cosine over the store
   └─▶ top-N ranked (file:lines + score + preview)
```

Five subcommands, all gated behind the feature flag **and** the runtime toggle:
- `embed-poc hello <text>` — single-vector smoke test (Phase 1)
- `embed-poc index <dir>` — chunk + embed + JSON store (Phase 2)
- `embed-poc inspect [--limit N]` — human-readable store dump
- `embed-poc search <query> [--top N]` — cosine top-N
- `embed-poc serve [--port 7877] [--ui ui/dist]` — `tiny_http` JSON server + optional React SPA

Plus one always-available toggle (no feature flag required):
- `ig emb [on|off|status]` — flip the runtime switch persisted in `~/.config/ig/embed.toml`. Fail-closed: if the config file is unreadable, embeddings stay off.

Why this is **not the default**:
- **Cost guard.** An indexing run on a 3 k-file repo costs ~$0.05; a runaway re-index in a CI loop could rack up real money. PMI/trigram are free.
- **Network dependency.** Each search is one round-trip to OpenAI (~200–800 ms). The trigram daemon answers in < 1 ms.
- **API-key handling.** The key lives in `~/.config/ig/config.toml` or `.env` (always gitignored, pre-commit hook blocks `sk-*` strings) — but most users don't have one and shouldn't have to.
- **Recall is similar at this scale.** On a 3 k-file repo, well-tuned PMI + BM25 (`ig --semantic --top 10`) catches most queries that dense embeddings catch. Embeddings start to dominate at 50 k+ files / multi-language polyglot repos.

The POC stays in-tree (gated) so users curious about embedding-based search can see exactly what an embedding *is* (1 536 floats, L2-normalised, 32×48 heatmap visualisable in the SPA), measure the latency/cost themselves, and decide whether to industrialise.

Read the full design + Phase 0–4 walkthrough at [`/docs/embeddings`](https://instant-grep.pulseview.app/docs/embeddings).

## Architecture

```
ig
├── index/          — Sparse n-gram index (build + query + overlay)
├── search/         — Indexed + brute-force search
│   └── rank.rs     — BM25 ranking (--top N, v1.10.0)
├── semantic/       — PMI cooccurrence tokenizer + builder (v1.10.0)
├── query/          — Regex → NgramQuery conversion
├── git.rs          — Token-compressed git proxy
├── rewrite.rs      — Command rewriting engine (exit codes 0/1/2/3)
├── gain.rs         — Token savings dashboard
├── tracking.rs     — JSONL history
├── discover.rs     — Session scanner for missed savings
├── setup.rs        — Universal AI agent configuration
│                     + managed-block markers in CLAUDE.md / AGENTS.md (v1.19.1)
│                     + --quiet flag for post-update re-sync (v1.19.2)
├── scoring.rs      — Layered Semantic Compression (entropy × weight × relevance)
├── delta.rs        — Git-aware delta reads (changed lines + enclosing context)
├── read.rs         — Smart file reading (full + signatures)
├── smart.rs        — 2-line file summaries + dir-aggregate mode (v1.10.0)
├── symbols.rs      — Symbol definition extraction
├── pack.rs         — Project context generator
├── ls.rs           — Compact directory listing
├── cache.rs        — XDG cache + gc/migrate (v1.15.0)
│                     + ensure_layout / projects/ / by-name/ / manifest (v1.19.0)
│                     + daemon log rotation (5 MB / 5 generations)
├── daemon.rs       — Single global Unix-socket server, multi-tenant LRU
│                     + .ig/ watcher for proactive seal reload (v1.16.0+, v1.18.0)
├── index/seal.rs   — 16-byte atomic publish marker (v1.18.0)
├── index/vbyte.rs  — Varbyte posting codec + masked PostingEntry (v1.17.1)
├── watch.rs        — File watcher + auto-rebuild
└── walk.rs         — Gitignore-aware walking
```

## FAQ

### Is `ig` a drop-in `ripgrep` replacement?

For search, yes. Regex syntax is 100 % `regex-syntax` (same crate `ripgrep` uses), and on the v1.11.0 benchmark `ig` and `rg 14.1.1` produce **byte-identical** match output on 5/5 patterns (file count + total matches). Flags differ in surface area — `ig` covers the common ones (`-i`, `-l`, `-c`, `-C N`, `-t TYPE`, `--json`) and adds index-only ones (`--top N`, `--semantic`, `--stats`).

### Does `ig` work without an index?

Yes. The first search on a new project triggers a background index build, and the query runs on a parallel scan in the meantime. Subsequent queries hit the index. You never need to call `ig index` manually — the daemon's filesystem watcher keeps it fresh.

### How big is the index?

Roughly **5–10 % of your codebase**. On a 1,609-file Laravel project: 38 MB. On a 3,084-file monorepo: 76 MB. Stored under `~/.cache/ig/projects/<hash>/`, never inside the repo.

### Does `ig` send any data over the network?

No. The default binary contains zero network code on the search path. The optional `embed-poc` subcommand (OpenAI embeddings) is **disabled at compile-time** unless you build with `--features embed-poc`, **and** disabled at runtime unless you flip `ig emb on`. Two opt-ins. Without them, every search is local, sub-ms, free.

### Which AI agents are supported?

`ig setup` configures **8 agents** out of the box: Claude Code, Codex CLI, OpenCode, Cursor, GitHub Copilot, Windsurf, Cline, and Gemini CLI. Each gets its rule file written + (when applicable) a PreToolUse hook installed to auto-rewrite `grep` / `cat` / `find` / `git` calls. 100 % idempotent.

### Linux, macOS, Windows?

Linux (x86_64 + ARM64) and macOS (x86_64 + Apple Silicon) are first-class — both ship a C shim + Rust backend with `install.sh` doing the placement automatically. **Windows is not supported** today (the daemon uses Unix domain sockets and `notify` filesystem watching paths that aren't portable). WSL2 works fine.

### How does this compare to `the_silver_searcher` (`ag`) or `ack`?

`ag` and `ack` predate `ripgrep` and are slower than both `rg` and `ig`. The trigram index in `ig` is the same algorithm class as **GitHub Code Search** (sparse n-grams from [danlark1/sparse_ngrams](https://github.com/danlark1/sparse_ngrams)) and Cursor's fast regex engine — it's a fundamentally different cost curve from grep-style scanners.

### Do I need to learn a new query syntax?

No. `ig "pattern" path` is identical to `rg "pattern" path`. The only thing you opt into is the index — and you do that just by running a search.

## Credits

- [danlark1/sparse_ngrams](https://github.com/danlark1/sparse_ngrams) — sparse n-gram algorithm
- [Cursor — Fast regex search](https://cursor.com/blog/fast-regex-search) — the inspiration
- [GitHub — The technology behind code search](https://github.blog/engineering/architecture-optimization/the-technology-behind-githubs-new-code-search/)
- [BurntSushi](https://github.com/BurntSushi) — `regex-syntax`, `ignore`, `memchr`

## License

MIT
