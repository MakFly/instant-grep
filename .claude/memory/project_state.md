---
name: ig-project-state
description: État complet du projet instant-grep (ig) v1.4.0 — features, benchmarks, architecture, tests
type: project
---

## instant-grep (ig) v1.4.0

### Benchmarks mesurés (réels, pas estimés)
- Token savings: 76% sur 806+ commandes (5.5MB saved)
- git status: -95% | git log: -89% | git show: -51%
- Context system: 3,828B/turn (vs 12,841B avant = -70%)
- Index build 1,609 files: 226ms | 3,084 files: 483ms
- Search: 23ms (1,609 files) | Daemon: sub-ms
- Symbols: 4,834 (Laravel 1,609 files) | 7,702 (monorepo 3,084 files)
- Explorer optimisé: 14 requêtes / 60s (vs 69 / 120s avant = -80%)

### Tests
- 160 tests Rust (cargo test)
- 63/65 integration tests (claude-test-full)
- CI: clippy 0 warnings, fmt clean, all platforms build

### CI/CD
- release.yml: cross-compile 4 targets, auto Homebrew formula update
- v1.4.0: https://github.com/MakFly/instant-grep/releases/tag/v1.4.0
