# ig v1.1.0 — Benchmark Results

**Date**: 2026-03-25
**Machine**: Apple M4 Max (14 cores), 36 GB RAM, macOS 15.5, NVMe SSD
**ig**: v1.1.0 (trigram-indexed regex search)
**ripgrep**: 15.1.0 (brute-force regex search)
**Method**: Best of 3 runs, wall time including process startup

---

## ig vs ripgrep — All Patterns, All Repos

| Repo | Files | Pattern | ig | rg | Winner | Speedup |
|------|------:|---------|---:|---:|--------|---------|
| trading-app | 11,350 | `function` (literal-common) | 33.2ms | 39.4ms | **ig** | 1.2x |
| trading-app | 11,350 | `class\s+\w+` (regex) | 28.8ms | 33.9ms | **ig** | 1.2x |
| trading-app | 11,350 | `deprecated` (literal-rare) | 21.4ms | 31.0ms | **ig** | 1.4x |
| trading-app | 11,350 | `zzzzxqwk` (no-match) | 20.2ms | 29.8ms | **ig** | 1.5x |
| trading-app | 11,350 | `import` | 24.3ms | 31.7ms | **ig** | 1.3x |
| trading-app | 11,350 | `todo` (-i, case-insensitive) | 20.5ms | 31.0ms | **ig** | 1.5x |
| distribution-app-v2 | 3,101 | `function` (literal-common) | 39.8ms | 43.3ms | **ig** | 1.1x |
| distribution-app-v2 | 3,101 | `class\s+\w+` (regex) | 32.9ms | 44.4ms | **ig** | 1.3x |
| distribution-app-v2 | 3,101 | `deprecated` (literal-rare) | 22.2ms | 36.7ms | **ig** | 1.7x |
| distribution-app-v2 | 3,101 | `zzzzxqwk` (no-match) | 20.7ms | 38.7ms | **ig** | 1.9x |
| distribution-app-v2 | 3,101 | `import` | 35.3ms | 41.8ms | **ig** | 1.2x |
| distribution-app-v2 | 3,101 | `todo` (-i, case-insensitive) | 21.4ms | 38.9ms | **ig** | 1.8x |
| headless-kit | 2,541 | `function` (literal-common) | 39.9ms | 46.5ms | **ig** | 1.2x |
| headless-kit | 2,541 | `class\s+\w+` (regex) | 29.6ms | 39.9ms | **ig** | 1.3x |
| headless-kit | 2,541 | `deprecated` (literal-rare) | 22.1ms | 35.6ms | **ig** | 1.6x |
| headless-kit | 2,541 | `zzzzxqwk` (no-match) | 20.4ms | 35.6ms | **ig** | 1.7x |
| headless-kit | 2,541 | `import` | 34.5ms | 41.4ms | **ig** | 1.2x |
| headless-kit | 2,541 | `todo` (-i, case-insensitive) | 21.3ms | 35.9ms | **ig** | 1.7x |
| distribution-app | 1,636 | `function` (literal-common) | 36.5ms | 34.7ms | rg | 1.1x |
| distribution-app | 1,636 | `class\s+\w+` (regex) | 28.3ms | 34.8ms | **ig** | 1.2x |
| distribution-app | 1,636 | `deprecated` (literal-rare) | 21.0ms | 30.2ms | **ig** | 1.4x |
| distribution-app | 1,636 | `zzzzxqwk` (no-match) | 21.0ms | 28.9ms | **ig** | 1.4x |
| distribution-app | 1,636 | `import` | 25.8ms | 31.3ms | **ig** | 1.2x |
| distribution-app | 1,636 | `todo` (-i, case-insensitive) | 21.2ms | 29.5ms | **ig** | 1.4x |
| boilerplater | 915 | `function` (literal-common) | 32.4ms | 41.3ms | **ig** | 1.3x |
| boilerplater | 915 | `class\s+\w+` (regex) | 25.9ms | 41.7ms | **ig** | 1.6x |
| boilerplater | 915 | `deprecated` (literal-rare) | 20.5ms | 43.1ms | **ig** | 2.1x |
| boilerplater | 915 | `zzzzxqwk` (no-match) | 20.5ms | 39.7ms | **ig** | 1.9x |
| boilerplater | 915 | `import` | 23.1ms | 41.3ms | **ig** | 1.8x |
| boilerplater | 915 | `todo` (-i, case-insensitive) | 20.5ms | 41.6ms | **ig** | 2.0x |

---

## ig vs ripgrep vs grep (from benchmark.sh --quick)

grep (BSD 2.6.0) was also tested but is orders of magnitude slower:

| Repo | Pattern | ig | rg | grep | ig vs grep |
|------|---------|---:|---:|-----:|------------|
| trading-app | `function` | 19ms | 25ms | 7,387ms | 387x |
| trading-app | `class\s+\w+` | 14ms | 24ms | 7,584ms | 542x |
| trading-app | `todo` (-i) | 7ms | 18ms | 13,144ms | 1,878x |
| trading-app | no-match | 7ms | 18ms | 6,293ms | 899x |
| trading-app | `deprecated` | 8ms | 18ms | 7,512ms | 916x |
| trading-app | `import` | 10ms | 19ms | 7,576ms | 758x |
| distribution-app-v2 | `function` | 25ms | 33ms | 63,272ms | 2,531x |
| distribution-app-v2 | `class\s+\w+` | 19ms | 32ms | 62,286ms | 3,278x |
| distribution-app-v2 | `todo` (-i) | 7ms | 29ms | 81,925ms | 11,703x |
| distribution-app-v2 | no-match | 7ms | 29ms | 50,254ms | 7,179x |

---

## Summary

**Score: ig 29 / 30 wins** against ripgrep (the single loss is `function` on distribution-app by 1.8ms).

### Key observations

- ig wins on **all pattern types**: literal, regex, case-insensitive, no-match, rare
- ig's advantage is most pronounced on **rare/no-match patterns** (1.4-2.1x) where the trigram index eliminates files without scanning them
- On **common patterns** (function, import), ig still wins (1.1-1.3x) thanks to lazy line_starts and the raised escape hatch threshold (85%)
- grep is 387-11,703x slower than ig — not a viable alternative for code search
- ripgrep's SIMD-optimized brute-force is fast, but cannot beat an index that skips 95%+ of files

### What changed in v1.1.0

1. **Lazy line_starts** — regex match check runs before building the line index. False-positive candidates bail immediately
2. **Early exit modes** — `-l` returns after first match, `-c` skips line_starts entirely
3. **Parallel fallback** — brute-force path now uses rayon (was sequential)
4. **Escape hatch raised** — from 60% to 85%, keeping the indexed path active for more patterns
5. **madvise hints** — `MADV_RANDOM` on postings.bin, `MADV_WILLNEED` on lexicon.bin
6. **Single-file search** — `ig "pattern" file.rs` now correctly scopes to that file
7. **Binary check optimized** — reads 8KB instead of entire file in fallback mode
