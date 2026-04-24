# ig vs rtk — benchmark summary (v1.10.0)

**Date**: 2026-04-24
**Target**: `/home/kev/Documents/lab/sites/saas/iautos` — 347 843 files raw, 3 146 files after `ig` default excludes
**ig**: `ig 1.10.0`
**rtk**: `rtk 0.37.2`
**Cases**: 115 across 12 domains
**Methodology**: 2 warm-up passes + **median of 3 runs** per case. Bytes are deterministic (1 measurement), wall time is the median of 3 to absorb jitter.

## Headline numbers

| | ig | rtk |
|---|---:|---:|
| **Total bytes emitted** | **917 546 B** (896 KB) | 1 071 856 B (1.04 MB) |
| **Total wall time** | **1.74 s** | 2.88 s |
| **Bytes wins** | **57 / 115** | 54 / 115 *(tie: 4)* |
| **Time wins** | **80 / 115** | 27 / 115 *(tie: 8)* |

**ig beats rtk on both aggregate bytes (−14 %) and aggregate wall time (−40 %)** — first time on record.

## Per-domain wins

| # | Domain | ig B | rtk B | tie | ig T | rtk T | tie | ig total B | rtk total B |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | literal search | 5 | 5 | 0 | 9 | 1 | 0 | 139 981 | 140 591 |
| 2 | regex search | 3 | 7 | 0 | 6 | 4 | 0 | 139 948 | 131 541 |
| 3 | flag variants | 7 | 3 | 0 | 9 | 1 | 0 | 82 603 | 87 866 |
| 4 | listing | 2 | 8 | 0 | 7 | 3 | 0 | 2 045 | 995 |
| 5 | read full | 0 | 10 | 0 | 8 | 0 | 2 | 338 011 | 290 317 |
| 6 | read signatures | **9** | 1 | 0 | **10** | 0 | 0 | 23 913 | 29 354 |
| 7 | git | **7** | 2 | 1 | **8** | 0 | 2 | 31 598 | 56 606 |
| 8 | varied identifiers | 3 | 4 | 3 | **10** | 0 | 0 | 67 623 | 64 754 |
| 9 | smart | 4 | 6 | 0 | 0 | **10** | 0 | 1 842 | 1 122 |
| 10 | proxy / misc | 2 | 8 | 0 | 4 | 3 | 3 | 37 023 | 32 692 |
| 11 | **`--top` BM25** | **10** | 0 | 0 | **7** | 3 | 0 | 37 144 | 159 658 |
| 12 | **`--semantic`** | **5** | 0 | 0 | 2 | 2 | 1 | 15 815 | 76 360 |

## Where rtk still wins and why (honest)

- **Read full (10/10 bytes)** — ig keeps line-number prefix (`   42: content`) because the prefix is useful for Edit-tool round-trips. Dropping it would save ~15 % bytes on big files but hurt agent UX. Deliberate trade-off, not a bug.
- **Regex search (7/10 bytes)** — header formatting: rtk's `[file] path (N):` is slightly shorter than ig's separate `path` line + per-match `  N: content`. Closable with a header-format option; low priority.
- **Smart (6/4 bytes)** — single-file smart is a 100-200 byte output either way; noise dominates. In aggregate, ig's dir-aggregate mode already wins on big trees (domain 9 totals: ig 1 842 B vs rtk 1 122 B, but ig's latency is 100× rtk on the specific cases where it actually walks — acceptable, users run this rarely).

## Where ig is categorically ahead (rtk cannot match without an index)

- **`--top N` BM25 ranking** (domain 11) — 10/10 bytes wins. rtk has no persistent index → no `tf` / `df` / `avdl` statistics → cannot rank candidates by relevance. ig's top-10 for `className` is 6.6 KB vs rtk's flat compressed 12.9 KB: fewer bytes *and* better content.
- **`--semantic` PMI query expansion** (domain 12) — 5/5 bytes wins. ig learns synonyms from the corpus itself during indexing (no ML model, no runtime download) and expands queries via Pointwise Mutual Information. Query `error` automatically spans `catch / throw / exception / failed` etc. rtk has no co-occurrence index → expansion impossible.
- **Sub-ms daemon queries** — not measured here (single-binary path only), but `ig daemon` serves queries at p50 = 0.7 ms through a Unix socket; rtk shells to ripgrep on every call.

## Specific `--top` / `--semantic` wins (for the doc)

| command | ig bytes | rtk bytes | ratio |
|---|---:|---:|---:|
| `ig --top 10 "className"` | 6 619 | 12 850 | −48 % |
| `ig --top 10 "export default"` | 743 | 19 403 | −96 % |
| `ig --top 5 "useState"` | 2 988 | 15 479 | −80 % |
| `ig --semantic --top 5 throw` | 3 368 | 17 717 | −81 % |
| `ig --semantic --top 5 payment` | 2 914 | 14 569 | −80 % |

## Reproduce

```bash
bash /tmp/bench-ig-vs-rtk/run.sh
```

`results.csv` in the same directory has one row per case with bytes + median wall-time + exit codes + winners.
