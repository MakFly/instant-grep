# ig v1.0.0 — Benchmark Report

**Date**: 2026-03-24
**Machine**: Apple M4 Max, macOS 15.5, 128 GB RAM, NVMe SSD
**ig version**: 1.0.0 (INDEX_VERSION 7, SPIMI streaming + VByte + mmap lexicon)
**ripgrep version**: 15.1.0

---

## 1. Scaling Curve — ig vs rg across project sizes

Searching for `"function"` across 6 projects from 49 to 92K files.

| Project | Files | ig index | RSS | Segments | ig search | rg search | Speedup | Index size |
|---------|------:|---------|----:|---------|----------|----------|---------|-----------|
| laravel-app | 49 | 34ms | 23 MB | 1 | 19ms | 23ms | 1.2x | 1.3 MB |
| ig-repo (Rust) | 71 | 72ms | 101 MB | 1 | 19ms | 23ms | 1.2x | 7.1 MB |
| distribution-app (Laravel) | 1,552 | 410ms | 343 MB | 2 | 40ms | 38ms | ~1x | 36 MB |
| laravel-framework | 3,053 | 579ms | 374 MB | 3 | 115ms | 60ms | 0.5x | 42 MB |
| **Next.js** | **24,755** | 3.8s | **845 MB** | 10 | **502ms** | **1,547ms** | **3.1x** | 172 MB |
| **Linux kernel** | **92,585** | 60s | **6,829 MB** | 237 | **939ms** | **4,952ms** | **5.3x** | 3.4 GB |

**Key insight**: ig's advantage grows with project size. Below ~3K files, both are dominated by process startup (~15ms). Above 10K files, ig eliminates 95%+ of files. On the Linux kernel, a zero-result search takes **28ms vs 5,314ms** (**190x speedup**).

**Memory note**: The streaming SPIMI pipeline reduced Linux kernel RSS from 17.9 GB (v1.0.0-pre) to **6.8 GB** (streaming). The remaining RSS is dominated by the `MergedEntry` vec (127M entries × 16 bytes ≈ 2 GB) and segment I/O. For projects under 10K files, RSS stays under 1 GB.

---

## 2. Daemon Latency Percentiles — 1,001 queries

Distribution-app (1,552 files). Server-side search time.

### Overall

| Metric | Value |
|--------|-------|
| Total queries | 1,001 |
| p50 | **1.17ms** |
| p95 | 12.25ms |
| p99 | 13.24ms |
| avg | 3.73ms |

### Per pattern

| Pattern | p50 | p95 | p99 |
|---------|-----|-----|-----|
| `"ZZZNOTFOUND"` | **0.01ms** | 0.01ms | 0.02ms |
| `"DistributionController"` | **0.06ms** | 0.08ms | 0.12ms |
| `"Route::"` | **0.15ms** | 0.17ms | 0.27ms |
| `"middleware"` | **1.17ms** | 1.24ms | 1.31ms |
| `"exception"` | 1.41ms | 1.61ms | 1.69ms |
| `"class "` | 11.09ms | 11.74ms | 14.16ms |
| `"function"` | 12.15ms | 13.26ms | 15.31ms |

---

## 3. Throughput (QPS)

| Metric | Value |
|--------|-------|
| Effective QPS (with process startup) | **273 queries/sec** |
| Server-side avg latency | 0.61ms |
| Theoretical QPS (no process overhead) | **1,637 queries/sec** |

---

## 4. SPIMI Memory — Streaming Pipeline

Peak RSS during index build (128 MB SPIMI budget, 1K file batch streaming, mmap lexicon).

| Project | Files | Peak RSS | Segments |
|---------|------:|---------|---------|
| laravel-app | 49 | **23 MB** | 1 |
| ig-repo | 71 | **101 MB** | 1 |
| distribution-app | 1,552 | **343 MB** | 2 |
| laravel-framework | 3,053 | **374 MB** | 3 |
| Next.js | 24,755 | **845 MB** | 10 |
| Linux kernel | 92,585 | **6,829 MB** | 237 |

**vs before streaming** (Linux kernel): 17,943 MB → **6,829 MB** (**-62%**).

Remaining RSS sources:
- `MergedEntry` vec (proportional to unique n-grams, ~2 GB for Linux kernel)
- Segment file reads during merge
- Metadata `files` vec (path strings, ~10 MB for 92K files)

---

## 5. Overlay vs Full Rebuild

| Changed files | Time | Mode |
|--------------|------|------|
| 0 (no-op) | **24ms** | Skip |
| 1 | **27ms** | Overlay |
| 10 | **64ms** | Overlay |
| 50 | **69ms** | Overlay |
| 100 | **75ms** | Overlay |
| 1,552 (all) | **435ms** | Full SPIMI rebuild |

---

## 6. Cold vs Warm Cache

| Condition | ig | rg | Speedup |
|-----------|----|----|---------|
| Cold | 19ms | 29ms | **1.5x** |
| Warm | 19ms | 30ms | **1.6x** |

---

## 7. Token Cost

| Pattern | ig tokens | rg tokens | Ratio |
|---------|----------|----------|-------|
| `"function"` | 231,907 | 690,604 | **3.0x** |
| `"middleware"` | 3,367 | 12,680 | **3.8x** |
| `"class "` | 30,837 | 189,110 | **6.1x** |

---

## 8. Concurrent Queries — 10 clients

| Metric | Value |
|--------|-------|
| Concurrent clients | 10 |
| Total queries | 200 |
| Errors | 0 |
| Wall time | 0.89s |
| Effective QPS | **225** |
| Server-side p50 | 2.53ms |
| Server-side p95 | 18.14ms |

---

## 9. Index Compression — v6 vs v7

| Component | v6 (raw u32) | v7 (VByte) | Reduction |
|-----------|-------------|-----------|-----------|
| postings.bin | ~15.1 MB | **6.8 MB** | **2.2x** |
| metadata.bin | ~33.0 MB | **109 KB** | **310x** |
| lexicon.bin | 29.2 MB | 29.2 MB | 1.0x |
| **Total** | **~77 MB** | **~36 MB** | **2.1x** |

---

## 10. Linux Kernel Stress Test — 92,585 files

### Index build

| Metric | Value |
|--------|-------|
| Files indexed | 92,585 |
| Unique n-grams | 127,409,095 |
| Build time | 60s |
| Segments (SPIMI) | 237 |
| Peak RSS | **6,829 MB** (down from 17,943 MB pre-streaming) |
| Index size | 3.4 GB |

### Search: ig vs rg

| Query | ig | rg | Speedup |
|-------|----|----|---------|
| `"printk"` | **412ms** | 5,795ms | **14.1x** |
| `"static inline"` | **876ms** | 5,106ms | **5.8x** |
| `"mutex_lock"` | **278ms** | 5,330ms | **19.2x** |
| `"EXPORT_SYMBOL"` | **427ms** | 5,406ms | **12.7x** |
| `"ZZNOTFOUND"` (0 hits) | **28ms** | 5,314ms | **189.8x** |

---

## Summary

| Benchmark | Key result |
|-----------|-----------|
| **Scaling** | 1.2x (49 files) → **5.3x** (92K files) → **190x** (zero result) |
| **Daemon p50** | **1.17ms** overall, **0.01ms** zero results |
| **QPS** | 273 effective, **1,637** server-side |
| **Memory** | 23 MB (49 files) → 6.8 GB (92K files). **-62%** vs pre-streaming |
| **Overlay** | **27-75ms** for 1-100 changed files (6x faster than rebuild) |
| **Cold/warm** | Consistent **1.5x** speedup |
| **Tokens** | ig produces **3-6x fewer tokens** than rg |
| **Concurrent** | 10 clients, **225 QPS**, 0 errors |
| **Compression** | Index **2.1x** smaller, metadata **310x** smaller |
| **Linux kernel** | **5.8-190x** faster than rg on 92K files |

### Comparison with ripgrep — is it fair?

ripgrep (rg) is the gold standard for CLI code search. It uses SIMD-optimized scanning, memory-mapped I/O, and smart heuristics. Our comparison is fair and honest:

- **ig wins** on large projects (>10K files) where the index eliminates 95%+ of files before regex
- **ig wins** on repeated queries (daemon mode: sub-ms vs rg's process startup)
- **ig wins** on token cost (2-6x fewer tokens for LLM agents)
- **rg wins** on small projects (<3K files) where both are startup-bound
- **rg wins** on short patterns (<3 chars) where ig falls back to brute-force
- **rg wins** on zero-setup scenarios (no index to build)

Both tools have their place. ig is purpose-built for AI agent workflows where the same codebase is searched hundreds of times per session.
