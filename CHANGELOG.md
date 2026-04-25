# Changelog

All notable changes to `instant-grep` are documented here. Format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions adhere to [SemVer](https://semver.org/).

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
