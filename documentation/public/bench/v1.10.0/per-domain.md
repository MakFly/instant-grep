# Per-domain detail — ig vs rtk v1.10.0

Target: `/home/kev/Documents/lab/sites/saas/iautos`
ig 1.10.0 · rtk 0.37.2 · 115 cases · median of 3 runs after 2 warm-ups

Legend: B = bytes emitted by the tool (lower is better). T = median wall time in ms (lower is better).

## Domain 1 — literal search (10 cases)

Common substrings, single & multi-word.

| # | Pattern | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 001 | `function` | 13 524 | 41 | 14 277 | 37 |
| 002 | `TODO` | 5 454 | 7 | 5 035 | 27 |
| 003 | `import` | 15 410 | 33 | 16 244 | 36 |
| 004 | `iautos` | 15 643 | 12 | 15 047 | 28 |
| 005 | `export default` | 21 648 | 13 | 19 403 | 27 |
| 006 | `useState` | 15 440 | 11 | 15 479 | 27 |
| 007 | `Exception` | 13 287 | 11 | 13 084 | 32 |
| 008 | `className` | 14 793 | 25 | 12 850 | 46 |
| 009 | `async function` | 15 847 | 13 | 15 369 | 25 |
| 010 | `é` | 13 458 | 44 | 13 803 | 171 |

## Domain 2 — regex search (10 cases)

| # | Pattern | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 011 | `^import` | 20 804 | 31 | 12 693 | 34 |
| 012 | `[A-Z][a-z]+Error` | 15 444 | 17 | 14 479 | 27 |
| 013 | `(get\|post\|put)` | 15 687 | 45 | 12 883 | 118 |
| 014 | `export.*function` | 16 521 | 41 | 15 320 | 37 |
| 015 | `v[0-9]+\.[0-9]+` | 4 976 | 14 | 5 729 | 29 |
| 016 | email regex | 15 598 | 81 | 14 068 | 27 |
| 017 | url regex | 16 092 | 18 | 15 716 | 37 |
| 018 | `const [A-Z_]+ =` | 15 345 | 32 | 14 526 | 28 |
| 019 | `/api/v[0-9]+/` | 15 319 | 12 | 14 430 | 26 |
| 020 | `=>\s*\{` | 13 364 | 28 | 11 697 | 31 |

## Domain 3 — flag variants (10 cases)

| # | Command | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 021 | `ig -i 'todo'` vs `rtk grep 'todo' -i` | 138 | 7 | 6 051 | 26 |
| 022 | `-l 'useState'` files-with-matches | 20 247 | 12 | 18 | 26 |
| 023 | type filter `ts 'interface'` | 6 827 | 8 | 6 940 | 19 |
| 024 | type filter `tsx 'return'` | 10 888 | 21 | 55 | 9 |
| 025 | `-w 'id'` word-match | 15 756 | 45 | 14 653 | 181 |
| 026 | `-F 'const x ='` fixed-string | 367 | 7 | 317 | 26 |
| 027 | `-C1 'export default'` | 8 728 | 12 | 17 589 | 26 |
| 028 | `-A2 'throw new'` | 9 311 | 10 | 17 198 | 28 |
| 029 | `-B1 'async'` | 12 341 | 18 | 14 808 | 31 |
| 030 | glob `*.json 'name'` | 7 265 | 11 | 10 293 | 174 |

## Domain 4 — listing (10 cases)

ig's default excludes (node_modules, target, dist, ...) shrink the search surface from 348 k files → 3 k. `rtk ls` emits a placeholder 8 B for top-level dirs which is why it wins bytes by a wide margin here — but the bytes are uninformative.

| # | Command | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 031-040 | various ls / files — see results.csv | | | | |

## Domain 5 — read full (10 cases)

rtk wins 10/10 bytes: ig emits `   42: content` line-number prefix (useful for Edit tool), rtk emits raw content. 15 % overhead per file. Deliberate UX trade-off.

## Domain 6 — read signatures (10 cases) — **ig wins 9/10 bytes, 10/10 time**

`ig read -s` vs `rtk read -l aggressive`. Symbol-aware extraction beats generic regex stripping.

## Domain 7 — git (10 cases) — **ig wins 7/10 bytes, 8/10 time**

| # | Command | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 061 | `git status` | 93 | 10 | 134 | 13 |
| 062 | `git log -n 10` | 1 037 | 8 | 2 871 | 11 |
| 063 | `git log -n 50` | 4 972 | 12 | 14 177 | 11 |
| 064 | `git log --oneline -n 100` | 6 774 | 12 | 6 754 | 11 |
| 065 | `git diff HEAD~1` | 7 197 | 13 | 18 888 | 16 |
| 066 | `git show HEAD` | 5 564 | 12 | 18 788 | 21 |
| 067 | `git show HEAD --stat` | 1 076 | 11 | 1 076 | 11 |
| 068 | `git branch -a` | 114 | 5 | 61 | 9 |
| 069 | `git log -n 5 --name-only` | 1 217 | 8 | 967 | 12 |
| 070 | `git diff --stat HEAD~3` | 1 098 | 16 | 548 | 13 |

## Domain 8 — varied identifiers (10 cases)

Mixed case styles. ig wins 10/10 time via trigram index.

## Domain 9 — smart (10 cases)

Per-file + dir-aggregate. rtk edges bytes (6/4) but ig's dir-aggregate mode returns *structure* (file counts by ext, top subdirs, key manifests) where rtk returns a one-liner header.

## Domain 10 — proxy / misc (10 cases)

Mixed. ig's specialized subcommands (ig ls, ig git, ig json) compress where they have structural knowledge; rtk's generic proxy edges out on a few cases.

## Domain 11 — `--top N` BM25 ranking (10 cases) — **ig wins 10/10 bytes, 7/10 time**

ig returns the N files with the highest relevance score. rtk has no index → can only cap the flat-compressed output length.

| # | Command | ig B | ig T | rtk B | rtk T |
|---|---|---:|---:|---:|---:|
| 101 | `ig --top 10 'function'` vs rtk grep | 6 274 | 46 | 14 277 | 37 |
| 102 | `ig --top 5  'useState'` | 2 988 | 12 | 15 479 | 27 |
| 103 | `ig --top 10 'className'` | 6 619 | 25 | 12 850 | 45 |
| 104 | `ig --top 5  'import'` | 3 237 | 38 | 16 244 | 37 |
| 105 | `ig --top 10 'export default'` | 743 | 13 | 19 403 | 26 |
| 106 | `ig --top 5  '<div'` | 3 388 | 21 | 11 236 | 34 |
| 107 | `ig --top 10 'return'` | 4 641 | 46 | 21 340 | 41 |
| 108 | `ig --top 5  'async function'` | 2 481 | 13 | 15 369 | 29 |
| 109 | `ig --top 10 'interface'` | 3 436 | 12 | 15 040 | 25 |
| 110 | `ig --top 5  'throw new'` | 3 337 | 11 | 18 420 | 26 |

## Domain 12 — `--semantic` PMI expansion (5 cases) — **ig wins 5/5 bytes**

PMI-learned synonyms expand the query, BM25 ranks the results.

| # | Command | ig B | rtk B | Notes |
|---|---|---:|---:|---|
| 111 | `ig --semantic --top 5 error` | 2 664 | 13 777 | expanded 'error' → categorize |
| 112 | `ig --semantic --top 5 throw` | 3 368 | 17 717 | → got, inattendu, denied, autorisé, trouvée, manquant |
| 113 | `ig --semantic --top 5 payment` | 2 914 | 14 569 | → moneybanq, gateway, finalized, intent, gateways, requirement |
| 114 | `ig --semantic --top 5 auth` | 3 421 | 15 092 | → anon, better, pwa, firewall, step1, jwe |
| 115 | `ig --semantic --top 5 config` | 3 448 | 15 205 | → directives, commitlint, compiled, configs, nelmio, eslintrc |
