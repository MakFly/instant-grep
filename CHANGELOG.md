# Changelog

All notable changes to `instant-grep` are documented here. Format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions adhere to [SemVer](https://semver.org/).

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
