# ig v2.3.0 vs RTK v0.42.2 — Benchmark Summary

**Date**: 2026-06-05
**Codebase**: instant-grep (Rust, ~90 source files)
**Platform**: Linux x86_64, ripgrep 15.1.0

## Totals — 21 cases

|                      | ig 2.3.0   | rtk 0.42.2 |
|----------------------|------------|------------|
| Total bytes emitted  | 144,247    | 170,726    |
| Total wall time      | 2,327 ms   | 2,323 ms   |
| Bytes wins           | 8 / 21     | 11 / 21    |
| Time wins            | 4 / 21     | 9 / 21     |

**ig emits 15.5% fewer bytes overall**, with identical wall-clock times (~110ms per command).

## Per-Category Analysis

### Search (6 cases)

ig's structural advantages (trigram index, BM25 ranking, PMI expansion) deliver **80-86% byte reductions** on --top and --semantic queries. RTK cannot match these — it has no index.

- `--top 5 filter`: ig 2,553 B vs rtk 16,088 B (**-84%**)
- `--semantic error`: ig 2,291 B vs rtk 16,384 B (**-86%**)
- `grep -c`, `grep -l`: RTK does not support these flags (returns 0/empty)

### File Reads (4 cases)

- **Full reads**: rtk smaller by ~15% (ig preserves line-number prefix for Edit tool precision — deliberate trade-off)
- **Signature reads**: ig **84% smaller** on main.rs (2,354 B vs 14,833 B) — ig extracts import+fn signatures, rtk strips bodies less aggressively

### Smart (2 cases)

- `rtk smart` only works on individual files, not directories
- ig supports directory-level smart summaries

### Git (3 cases)

- ig wins on status (-6%) and log (-27%)
- rtk wins on diff (-21%)

### Listing (2 cases)

- rtk slightly smaller (~10%) — marginal

### Tools (4 cases)

- rtk wins on env (-69%) and deps (-35%)
- ig wins on diff (-13%)

## RTK Functional Gaps

Flags that RTK's grep wrapper does not support:
- `-c` (count per file) — returns "0" regardless
- `-l` (files-with-matches) — returns empty
- `-i` (case-insensitive) — not passed through to rg

RTK smart does not support directory arguments (fails with "Is a directory").

## Key Takeaway

On shared territory, the tools are comparable. ig's structural moat (trigram index → BM25 + semantic + signatures) accounts for the aggregate byte advantage. These capabilities require a persistent index that RTK does not have.
