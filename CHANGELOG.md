# Changelog

All notable changes to `instant-grep` are documented here. Format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions adhere to [SemVer](https://semver.org/).

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
