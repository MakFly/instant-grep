# SPEC: RTK feature-iso (and better) for `ig` v2.x

> Status: Draft 1 — code-grade implementation plan
> Author: Claude Opus 4.7 (1M context), commissioned by kevin
> Date: 2026-05-19
> Branch baseline: `refactor/remove-daemon-rtk-iso` (commit `f716f85`, post-daemon-removal)
> Scope: bring `ig` to functional parity with [rtk-ai/rtk](https://github.com/rtk-ai/rtk) `v0.34.3`, *plus* differentiate via the trigram index, while keeping the single-binary, no-daemon posture of v2.0.0.
> Out of scope: writing any Rust in `src/`. This document is the plan only.

---

## 0. Reading guide

- "ig:<path>" = absolute path under `/Users/kevin/Documents/lab/sandbox/instant-grep/`.
- "rtk:<path>" = path under `/tmp/rtk-research/` (clone of `rtk-ai/rtk@master` pulled at draft time).
- All file paths in the *Plan PR-by-PR* and *Architecture* sections are absolute.
- Effort sizing: S = ≤1 day, M = 2–4 days, L = 1–2 weeks of focused work for one engineer who already knows the codebase.

The full list of RTK source files surveyed for this spec:

- `rtk:src/main.rs` (3,175 LOC — clap enum, routing, fallback)
- `rtk:src/cmds/{git,js,python,rust,go,ruby,dotnet,cloud,system,jvm}/*.rs` (42 command modules)
- `rtk:src/filters/*.toml` (60 declarative TOML filters)
- `rtk:src/core/{tracking,telemetry,toml_filter,runner,stream,tee,filter,config,utils,display_helpers}.rs`
- `rtk:src/hooks/{init,rewrite_cmd,permissions,integrity,trust,hook_check,hook_audit_cmd,hook_cmd,verify_cmd}.rs`
- `rtk:src/discover/{mod,lexer,provider,registry,report,rules}.rs`
- `rtk:src/learn/{mod,detector,report}.rs`
- `rtk:src/analytics/{mod,gain,cc_economics,ccusage,session_cmd}.rs`
- `rtk:src/parser/{mod,formatter,types}.rs`
- `rtk:hooks/{claude,copilot,cursor,codex,gemini,opencode,hermes,windsurf,cline,kilocode,antigravity}/`
- `rtk:openclaw/`
- `rtk:docs/contributing/{ARCHITECTURE,TECHNICAL,CODING_PRACTICES}.md`
- `rtk:docs/TELEMETRY.md`

---

## 1. Inventory of RTK

### 1.1 Top-level shape

RTK is a single `~4 MB` Rust binary (`rtk`). One process per invocation, no daemon, no IPC.
Pipeline per command:

```
Clap parse → maybe telemetry ping → hook integrity check → match Commands → cmds/<mod>::run() →
  Command::new(tool).output() → strip_ansi/parse/regroup → print → tracking::record (SQLite) →
  optional tee on failure → propagate exit code
```

Fallback path (`run_fallback`, `rtk:src/main.rs:1146`): if Clap rejects the input (unknown subcommand), RTK looks the raw command up in the **TOML filter engine** (`rtk:src/core/toml_filter.rs`), runs it captured, applies the matching filter, tracks savings; if no filter matches it does a pure passthrough with `Stdio::inherit`.

Two global flags injected on every command:

| Flag | Effect |
|------|--------|
| `-v / -vv / -vvv` | `Count` arg; level 1 prints debug, 2 prints executed cmd, 3 prints raw output to stderr |
| `-u, --ultra-compact` | Swap human strings for ASCII icons + single-line summaries |
| `--skip-env` | Bypass env-prefix preservation in some commands (added late) |

### 1.2 Hard-coded `Commands` enum (extracted from `rtk:src/main.rs`)

The enum is enormous (lines 76 → ~1120). Below is the catalogue *grouped by category*, with the filter strategy each module uses. Strategy codes are from the RTK taxonomy (`rtk:docs/contributing/ARCHITECTURE.md:199-309`):

`STATS` = stats extraction · `ERR` = error-only · `GROUP` = group by pattern · `DEDUP` = dedup with counts ·
`STRUCT` = structure-only · `CODE` = code-aware stripping · `FAIL` = failure-focus · `TREE` = tree compression ·
`PROG` = progress filter · `JSONTXT` = json+text dual · `STATE` = state machine · `NDJSON` = streaming NDJSON ·
`TOML` = declarative TOML filter only

#### 1.2.1 Git / VCS (`rtk:src/cmds/git/`)

| Subcommand | Module | Strategy | RTK savings claim |
|---|---|---|---|
| `git status` | `git.rs::run_status` (porcelain v1, branch-aware) | STATS+STATE | 80% |
| `git log` | `git.rs::run_log` (parses `--oneline`-style or normal) | STATS | 80% |
| `git diff` | `git.rs::run_diff` + `compact_diff` | STATS+CODE | 75-80% |
| `git show` | `git.rs::run_show` (handles blob vs commit) | STATS | 80% |
| `git add/commit/push/pull/fetch` | dedicated `run_*` funcs | "ok"-confirmations | 59-92% |
| `git branch` / `git worktree` / `git stash` | filter list outputs | STATS | 75-80% |
| `git -C/-c/--git-dir/--work-tree …` | every subcommand respects 6 global git opts (`rtk:src/main.rs:126-156`) | passthrough preservation | — |
| `gh pr/issue/run/repo/api/release …` | `gh_cmd.rs` (1k+ LOC) — calls `gh … --json` and re-emits compact view, strips markdown noise from PR bodies (HTML comments, badge lines, image-only lines, hrules, ≥3 blank lines collapsed) | JSONTXT+CODE | 26-87% |
| `glab mr/issue/ci/pipeline/api/release …` | `glab_cmd.rs` | mirrors `gh_cmd` | — |
| `gt …` (Graphite CLI) | `gt_cmd.rs` | STATS | — |
| `diff <a> <b>` | `diff_cmd.rs` | CODE | — |

#### 1.2.2 JS / TS (`rtk:src/cmds/js/`)

| Tool | Module | Strategy | Notes |
|---|---|---|---|
| `jest` | (rolled into `vitest_cmd`-style) | FAIL+STATE | 99.5% on green |
| `vitest` | `vitest_cmd.rs` | JSONTXT+FAIL | injects `--reporter=json`, falls back to regex+state machine if JSON missing; uses `extract_json_object` to survive `pnpm`/`dotenv` prefixes |
| `playwright` | `playwright_cmd.rs` | FAIL+STATE | 94% |
| `tsc` | `tsc_cmd.rs` | GROUP (by file then error code) | 83% |
| `eslint` / `biome` / `lint` | `lint_cmd.rs` | GROUP (by rule, by file) | 84% |
| `prettier --check` | `prettier_cmd.rs` | files-needing-fmt only | 70% |
| `next build` | `next_cmd.rs` | extract route table, drop chunks | 87% |
| `prisma` | `prisma_cmd.rs` | strip ASCII art | 88% |
| `pnpm` (`exec/install/list/outdated/run`) | `pnpm_cmd.rs` | TREE+STATS | 70-90% |
| `npm run/x/exec/rum/urn` | `npm_cmd.rs` | passthrough w/ filter | — |
| `npx <cmd>` | `npm_cmd.rs::npx` | re-dispatches to underlying tool's filter | — |

**Critical infrastructure**: package-manager auto-detect (`is_pnpm = pnpm-lock.yaml exists`, `is_yarn = yarn.lock`, fallback `npx --no-install`) used by every JS module. Documented at `rtk:docs/contributing/ARCHITECTURE.md:577-617`.

#### 1.2.3 Python (`rtk:src/cmds/python/`)

| Tool | Module | Strategy |
|---|---|---|
| `ruff check` | `ruff_cmd.rs` | JSON (`--output-format=json`), group-by-rule |
| `ruff format` | `ruff_cmd.rs` | text summary line |
| `pytest` | `pytest_cmd.rs` | STATE machine (Header→TestProgress→Failures→Summary), injects `--tb=short -q -rxX`, surfaces XFAIL/XPASS |
| `pip list/outdated/install/show` | `pip_cmd.rs` | JSON + uv auto-detect (`pip3? | uv pip`) |
| `mypy` | `mypy_cmd.rs` | GROUP by file |

Notable: `(python[0-9.]*\s+-m\s+)?pytest` regex pattern so `python -m pytest` and `python3.11 -m pytest` both match.

#### 1.2.4 Rust (`rtk:src/cmds/rust/`)

| Tool | Module | Strategy |
|---|---|---|
| `cargo build/check/clippy/fmt/install/test` | `cargo_cmd.rs` + `runner.rs` | FAIL+GROUP+STATE, handles json-rendered diagnostics |
| `err <cmd>` | `runner.rs::run_err` (generic) | regex match `error/warning` then context |

#### 1.2.5 Go (`rtk:src/cmds/go/`)

| Tool | Module | Strategy |
|---|---|---|
| `go test` | `go_cmd.rs::run_test` | NDJSON streaming (`-json`), aggregates `Action=run/fail/pass/output` per package, interleaved |
| `go build` | `go_cmd.rs::run_build` | error-only text |
| `go vet` | `go_cmd.rs::run_vet` | text |
| `golangci-lint run` | `golangci_cmd.rs` | JSON → group-by-linter, with custom global-opt handling (`-c/--color/--config/...`) |

#### 1.2.6 Ruby (`rtk:src/cmds/ruby/`)

| Tool | Module | Strategy |
|---|---|---|
| `rake test` / `rails test` | `rake_cmd.rs` | STATE (minitest) — "ok rake test: N runs, M failures" or numbered details |
| `rspec` | `rspec_cmd.rs` | inject `--format json`, fallback text state machine, strip Spring/SimpleCov/DEPRECATION/Capybara noise |
| `rubocop` | `rubocop_cmd.rs` | inject `--format json`, group by cop+severity, **skip** JSON injection in `-a/-A` autocorrect modes |
| `bundle install` | TOML filter (`bundle-install.toml`) | strip "Using" lines, short-circuit "ok bundle: complete" |

Shared util `ruby_exec(tool)` auto-detects `bundle exec` when `Gemfile` exists.

#### 1.2.7 .NET (`rtk:src/cmds/dotnet/`)

| Tool | Module | Strategy |
|---|---|---|
| `dotnet build/test/format` | `dotnet_cmd.rs` | FAIL+GROUP |
| `dotnet test --logger trx` | `dotnet_trx.rs` | XML parse (uses `quick-xml`) |
| `dotnet build -bl` | `binlog.rs` | binlog parse |
| `dotnet format` | `dotnet_format_report.rs` | text summary |

#### 1.2.8 Cloud / Infra (`rtk:src/cmds/cloud/`)

| Tool | Module | Strategy |
|---|---|---|
| `aws <service> <verb>` | `aws_cmd.rs` | huge: STS one-liner, EC2 compact, Lambda strips secrets, S3 ls truncated→tee, dynamodb unwraps type annotations, IAM strips policy docs, CloudFormation failures-first, logs `get-log-events` timestamp+msg only |
| `docker ps/images/logs/run/exec/build`, `docker compose ps/logs/build`, `kubectl get/logs/describe/apply` | `container.rs` | TREE+DEDUP (logs) |
| `psql` | `psql_cmd.rs` | strip banners, pretty table compact |
| `curl <url>` | `curl_cmd.rs` | truncate body → tee, body length header |
| `wget <url>` | `wget_cmd.rs` | strip progress bars (PROG) |

#### 1.2.9 System / Files (`rtk:src/cmds/system/`)

| Command | Module | Strategy |
|---|---|---|
| `ls` | `ls.rs` | TREE (`+-- src/ (8 files)`) — 65-80% |
| `tree` | `tree.rs` | TREE |
| `read <file>` | `read.rs` | CODE with FilterLevel::{None,Minimal,Aggressive} (strip comments → strip bodies). Languages: Rust, Python, JS/TS, Go, C, C++, Java |
| `find` | `find_cmd.rs` | GROUP by dir |
| `grep` | `grep_cmd.rs` | GROUP by file/rule/error code — 75% |
| `json` | `json_cmd.rs` | STRUCT (keys + types, no values) |
| `env` | `env_cmd.rs` | filter pattern, mask sensitive values (`-f AWS` etc.) |
| `deps` | `deps.rs` | summarize package.json/Cargo.toml/go.mod/requirements.txt |
| `log <file>` | `log_cmd.rs` | DEDUP (consecutive identical lines → `(×N)`) |
| `summary <long cmd>` | `summary.rs` | heuristic summary |
| `format` | `format_cmd.rs` | run formatter, show diff stats |
| `wc` | `wc_cmd.rs` | passthrough+truncate |
| `local-llm` | `local_llm.rs` | ollama wrapper |
| `pipe` | `pipe_cmd.rs` | run stdin → filter |

#### 1.2.10 Data / Meta

| Command | Module | Notes |
|---|---|---|
| `proxy <cmd>` | `rtk:src/main.rs ~2245` | raw passthrough through pipes with stdout/stderr cap (`CAP: 1_048_576`), tracks as 0% savings |
| `run <cmd>` | alias of `proxy` (or filter-aware) | — |
| `test <cmd>` | generic test wrapper | wraps any cmd, exits with same code, hides stdout if green |
| `err <cmd>` | filter `error|warning` lines + ctx | — |
| `summary <cmd>` | heuristic | — |
| `init` | `hooks/init.rs` (5,637 LOC!) | huge — per-agent hook installer + RTK.md/AGENTS.md/GEMINI.md/.cursor/.windsurfrules/.clinerules/.kilocode injectors, dry-run, --hook-only, --claude-md, --auto-patch, --no-patch, --show, --uninstall |
| `verify` | `verify_cmd.rs` | runs TOML inline tests |
| `hook` | sub-enum `HookCommands` | low-level rewrite, default agent `claude` |
| `hook-audit` | `hook_audit_cmd.rs` | scans Claude Code sessions, reports hook health for last N days (default 7) |
| `discover` | `discover/mod.rs` | scans `~/.claude/projects/<encoded>/*.jsonl` for Bash tool calls, classifies via `discover::registry::classify_command`, reports supported/unsupported/RTK_DISABLED |
| `learn` | `learn/mod.rs` + `learn/detector.rs` | detect CLI-correction patterns ("user fixed agent's command") |
| `session` | `analytics/session_cmd.rs` | per-session RTK adoption % |
| `gain` | `analytics/gain.rs` | dashboard: summary, --graph, --history, --daily/weekly/monthly, --all, --format json, --failures |
| `cc-economics` | `analytics/cc_economics.rs` | $$ saved estimate using token pricing |
| `ccusage` | `analytics/ccusage.rs` | per-account Claude usage scrape |
| `telemetry {enable,disable,status,forget}` | `core/telemetry_cmd.rs` | RGPD-style consent flow |
| `rewrite <cmd>` | `hooks/rewrite_cmd.rs` | hook-internal — exit codes 0/1/2/3 mean allow/passthrough/deny/ask |
| `tee {list,show,clear}` | `core/tee.rs` | inspect saved raw outputs |
| `gc` / `cache-ls` | absent in RTK | (ig-specific, kept) |

### 1.3 RTK TOML filter schema (`rtk:src/core/toml_filter.rs` + `rtk:src/filters/*.toml`)

```toml
schema_version = 1

[filters.<id>]
description = "..."
match_command = "^my-tool\\s+build"          # regex anchored on full command line
strip_ansi = true
strip_lines_matching = ["^\\s*$", "^Downloading"]
keep_lines_matching = [...]                  # alternative whitelist
match_output = [                             # short-circuit: if output matches, return msg
  { pattern = "^Compilation failed", message = "ok: compile error" },
]
replace = [                                  # regex sub, line-by-line, chainable
  { find = "^node_modules/.*", with = "" },
]
truncate_lines_at = 200                      # per-line char cap
head_lines = 30
tail_lines = 20
max_lines = 40                               # absolute cap (applied LAST)
on_empty = "my-tool: ok"                     # if everything got filtered out

[[filters.<id>.tests]]                       # inline golden tests — picked up by `rtk verify`
input = """..."""
expected = "..."
```

Lookup priority (`rtk:src/core/toml_filter.rs:1-25`):

1. `.rtk/filters.toml` (project-local, committable, **trusted** by default? — see §6)
2. `~/.config/rtk/filters.toml` (user global)
3. `src/filters/*.toml` concatenated into the binary via `build.rs`
4. Passthrough (no match)

Env overrides: `RTK_NO_TOML=1` bypass, `RTK_TOML_DEBUG=1` traces match.

Built-in filter list (60 files, mostly catch-all for tools that don't get a dedicated Rust module):
ansible-playbook, basedpyright, biome, brew-install, bundle-install, composer-install, df, dotnet-build, du, fail2ban-client, gcc, gcloud, gradle, hadolint, helm, iptables, jira, jj, jq, just, liquibase, make, markdownlint, mise, mix-compile, mix-format, mvn-build, nx, ollama, oxlint, ping, pio-run, poetry-install, pre-commit, ps, quarto-render, rsync, shellcheck, shopify-theme, skopeo, sops, spring-boot, ssh, stat, swift-build, systemctl-status, task, terraform-plan, tofu-fmt, tofu-init, tofu-plan, tofu-validate, trunk-build, turbo, ty, uv-sync, xcodebuild, yadm, yamllint.

### 1.4 `rtk init` workflow (`rtk:src/hooks/init.rs` — 5,637 LOC)

Per-agent state machine. For each supported `AgentTarget`, the installer:

1. Detects whether the agent's home dir exists or its CLI is on `$PATH` (e.g. `~/.claude/` or `which claude`).
2. Writes the embedded hook script (e.g. `rtk:hooks/claude/rtk-rewrite.sh`) to the agent's hooks dir.
3. Computes its SHA-256 and stores it under `~/.local/share/rtk/integrity/<agent>.sha256` for future verification (`hooks/integrity.rs`).
4. Patches the agent's settings file (`~/.claude/settings.json`, `~/.cursor/hooks.json`, `~/.gemini/settings.json`, etc.) to register the hook.
5. Writes an awareness markdown (`RTK.md`, `~/.codex/AGENTS.md`, `~/.gemini/GEMINI.md`, `.windsurfrules`, `.clinerules`, `.kilocode/rules/rtk-rules.md`, `.agents/rules/antigravity-rtk-rules.md`).
6. Optionally writes both project-local `.rtk/filters.toml` and user-global `~/.config/rtk/filters.toml` templates.
7. Triggers consent flow for telemetry (interactive prompt; `--auto-patch` skips it but never auto-opts-in).

Modes (clap flags, `rtk:src/main.rs:331-380`):
- `--agent <claude|copilot|cursor|gemini|codex|windsurf|cline|opencode|hermes|kilocode|antigravity>` — pick the integration
- `-g, --global` — write to user home rather than project
- `--copilot`, `--gemini`, `--codex` — shortcut flags
- `--claude-md` — write the legacy full markdown instead of slim
- `--hook-only` — skip the markdown
- `--auto-patch` / `--no-patch` — patch settings.json without prompt, or skip patch
- `--dry-run` — preview only
- `--show` — verify installation
- `--uninstall` — remove everything for that agent

Sentinel format used in markdown injection (so re-install is idempotent and can be auto-updated):

```
<!-- rtk-instructions v2 -->
... rendered content ...
<!-- /rtk-instructions -->
```

Per-agent JSON formats:
- Claude Code: `settings.json` → `hooks.PreToolUse[].hooks[]={ type: "command", command: "<sh>", timeout: 5 }`
- Cursor: `hooks.json` → `preToolUse`
- Gemini CLI: `settings.json` → `BeforeTool`
- OpenCode: TS plugin in `~/.config/opencode/plugin/rtk.ts` exporting `tool.execute.before`
- OpenClaw: TS plugin `before_tool_call`
- Hermes: Python plugin adapter at `~/.hermes/plugins/rtk-rewrite/__init__.py`
- Windsurf / Cline / Kilo Code / Antigravity: project-scoped markdown rules only (no hook API)

### 1.5 SQLite history schema (`rtk:src/core/tracking.rs:660-720`)

```
~/.local/share/rtk/tracking.db    (linux)
~/Library/Application Support/rtk/tracking.db    (macos)
%APPDATA%\rtk\tracking.db    (windows)

CREATE TABLE commands (
    id              INTEGER PRIMARY KEY,
    timestamp       TEXT NOT NULL,        -- RFC3339 UTC
    original_cmd    TEXT NOT NULL,
    rtk_cmd         TEXT NOT NULL,
    input_tokens    INTEGER NOT NULL,
    output_tokens   INTEGER NOT NULL,
    saved_tokens    INTEGER NOT NULL,
    savings_pct     REAL NOT NULL,
    exec_time_ms    INTEGER DEFAULT 0,    -- since v0.7.1
    project_path    TEXT                  -- nullable (project-scoped queries via GLOB)
);
-- auto-cleanup on every INSERT:
DELETE FROM commands WHERE timestamp < datetime('now', '-90 days');
```

Token estimation heuristic: `ceil(text.len() / 4.0)`. Single-threaded with a `Mutex<Option<Tracker>>` future-proof guard.

A second pinger marker file lives next to the DB: `~/.local/share/rtk/.telemetry_last_ping` (mtime-based 23h gate). Device hash: `SHA-256(per-device random salt)` stored once in `~/.local/share/rtk/.device_salt`.

### 1.6 Telemetry payload (full field list)

See `rtk:docs/TELEMETRY.md` (350+ lines). Categories: Identity (1 field), Environment (4), Usage volume (6), Quality (4), Ecosystem (1), Retention (2), Economics (2), Adoption (2), Configuration (3), Feature meta-usage (1). 26 fields total in the JSON POST. Endpoint configured at build time via `RTK_TELEMETRY_URL`/`RTK_TELEMETRY_TOKEN` env vars (none compiled in by default → silent no-op).

Consent flow: stored in `~/.config/rtk/config.toml::[telemetry]{ consent_given, enabled }`. `consent_given` is tri-state (`Some(true)/Some(false)/None`); only `Some(true) && enabled` triggers a ping. Env opt-out: `RTK_TELEMETRY_DISABLED=1`. Commands: `rtk telemetry {status,enable,disable,forget}`. `forget` also deletes the device salt and (best-effort) sends a server-side erasure request.

### 1.7 Discover (`rtk:src/discover/mod.rs` + `provider.rs`)

Scans `~/.claude/projects/<encoded-cwd>/*.jsonl` (Claude Code session files). For each line, if `type=="tool_use" && name=="Bash"`, extract the command string and the `output_len` from the matching `tool_result`. Then for every shell segment (split on `&& || ; |`):

1. Strip RTK_DISABLED env prefix; if it wraps a Supported command → bump the `rtk_disabled` counter.
2. `classify_command()` → `Supported | Unsupported | Ignored`.
3. Bucket by `rtk_equivalent`, sum token estimates (real `output_len/4` if available, else `category_avg_tokens(category, subcmd)` fallback).

Report shape (`rtk:src/discover/report.rs`): supported list (by savings desc), unsupported list (by count desc), RTK_DISABLED examples (top 5), parse_errors, agent integration status. Output formats: text + JSON. CLI: `rtk discover [--project P | --all] [--since N] [--limit N] [--format json] [-v]`.

### 1.8 Learn (`rtk:src/learn/{detector,report}.rs`)

Detects **CLI correction patterns** in agent sessions: cases where the agent ran `cmd-A`, got a non-zero exit, then ran `cmd-A'` that succeeded. Aggregates `cmd-A → cmd-A'` transitions to surface unstable invocations. Output: text/json table of top corrections with confidence scores. CLI: `rtk learn [--since N] [--limit N]`.

### 1.9 Hidden mechanics worth capturing

- **Lexer-driven shell tokenizer** (`rtk:src/discover/lexer.rs`) — one-pass state machine yielding `Arg/Operator/Pipe/Redirect` tokens with byte offsets. Used by `rewrite_compound` and `strip_trailing_redirects` so quoting/escapes/heredocs are handled once. `<<` heredocs and `$((` arithmetic short-circuit the rewriter to `None` (safer to passthrough).
- **Pipe asymmetry**: only the LHS of `|` is rewritten. `find`/`fd` upstream of a pipe is **never** rewritten (their output format would break `xargs`). Operators `&&/||/;` rewrite both sides.
- **Env-prefix double pass**: `classify_command` strips env vars to match patterns; `rewrite_segment` extracts them again to re-prepend after rewrite (so `GIT_SSH_COMMAND="..." git push` round-trips).
- **Permission verdicts** (`rtk:src/hooks/permissions.rs`): `Allow/Ask/Deny/Default` map to exit codes `0/3/2/3`. SECURITY: `Default` MUST be 3 (ask), never 0 — issue #1155. Hard-coded deny rules: `git reset --hard`, `git clean -fd`, `rm -rf`. Hard-coded ask: `git push --force`.
- **Integrity check** (`rtk:src/hooks/integrity.rs`): on every operational command, RTK re-hashes the installed hook script and warns (rate-limited 1/day) if the SHA-256 drifts from the recorded value — defends against a stale/tampered hook.
- **`hook_check::maybe_warn`** runs once per day to nudge users whose hook is older than the binary.
- **`is_operational_command`** (`rtk:src/main.rs:2463`) — gates which commands trigger telemetry + integrity check (e.g. `gain`/`telemetry` themselves don't, to avoid recursion).
- **`run_fallback` capture cap**: 1 MiB hard cap on stdout buffering before forced flush; protects against memory blowup on rogue commands.
- **Hermes adapter** (`rtk:hooks/hermes/rtk-rewrite/__init__.py` runtime + `rtk:hooks/hermes/`): mutates `terminal.command` field via the plugin manifest API, since Hermes has no hook system.
- **`openclaw` plugin** (`rtk:openclaw/`): TS plugin published separately as `openclaw plugins install ./openclaw`.

---

## 2. Gap analysis

Legend: ✅ exists · 🟡 partial · ❌ missing · ⚙️ requires rework

| RTK command | RTK module | ig today | Gap |
|---|---|---|---|
| `git status/log/diff/show/add/commit/push/pull/branch/fetch/stash/worktree` | `rtk:src/cmds/git/git.rs` | 🟡 `ig:src/git.rs` (commands exist via `Commands::Git`) — coverage of subcommands unknown, no `-C/-c/--git-dir` handling visible | Extend; add 6 git global opts; add `worktree/stash/fetch/pull` ergonomics |
| `gh pr/issue/run/repo/api/release` | `rtk:src/cmds/git/gh_cmd.rs` | ❌ | Add `ig gh` subcommand + markdown noise stripper |
| `glab` | `rtk:src/cmds/git/glab_cmd.rs` | ❌ | Add `ig glab` (mirrors gh) |
| `gt` (Graphite) | `rtk:src/cmds/git/gt_cmd.rs` | ❌ | Add `ig gt` |
| `diff <a> <b>` | `rtk:src/cmds/git/diff_cmd.rs` | ✅ `ig:src/cmds/diff_cmd.rs` | Verify parity, add CODE-aware mode |
| `jest/vitest/playwright` | `rtk:src/cmds/js/{vitest_cmd,playwright_cmd}.rs` | 🟡 `ig:src/cmds/test_runner.rs` auto-detects but routes through generic `run` filter | Add dedicated `JSON+state-machine` parsers (vitest, jest, playwright) — use shared `parser::OutputParser` trait |
| `tsc` | `rtk:src/cmds/js/tsc_cmd.rs` | ❌ | New |
| `eslint/biome/lint` | `rtk:src/cmds/js/lint_cmd.rs` | 🟡 covered by `ig:filters/eslint-json.toml` + `lint-tools.toml` TOMLs | Add Rust GROUP-by-rule parser (richer than TOML) |
| `prettier --check` | `rtk:src/cmds/js/prettier_cmd.rs` | ❌ | New, small |
| `next build` | `rtk:src/cmds/js/next_cmd.rs` | ❌ | New |
| `prisma` | `rtk:src/cmds/js/prisma_cmd.rs` | ❌ | New |
| `pnpm/npm/npx` ecosystem dispatch | `rtk:src/cmds/js/{pnpm_cmd,npm_cmd}.rs` | 🟡 only TOML filters (`pnpm.toml`, `js.toml`) | Add package-manager detect util + Rust filters |
| `ruff check/format` | `rtk:src/cmds/python/ruff_cmd.rs` | 🟡 `ig:filters/ruff-extended.toml` | Add Rust JSON path |
| `pytest` | `rtk:src/cmds/python/pytest_cmd.rs` | 🟡 routed via `ig test` (TOML in `python.toml`) | Add Rust state-machine parser, XFAIL/XPASS surfacing |
| `pip list/outdated/install/show` | `rtk:src/cmds/python/pip_cmd.rs` | ❌ | New (with uv auto-detect) |
| `mypy` | `rtk:src/cmds/python/mypy_cmd.rs` | 🟡 (in `lint-tools.toml`) | Add GROUP parser |
| `cargo build/check/clippy/fmt/install/test` | `rtk:src/cmds/rust/cargo_cmd.rs` | 🟡 `ig:filters/cargo.toml` + `cargo-audit.toml` | Add Rust diagnostics-grouped parser (JSON) |
| `go test/build/vet` | `rtk:src/cmds/go/go_cmd.rs` | 🟡 `ig:filters/go-test.toml` + `go.toml` | Add NDJSON streaming parser |
| `golangci-lint run` | `rtk:src/cmds/go/golangci_cmd.rs` | ❌ | New JSON parser with global-opt awareness |
| `rake test/rails test` | `rtk:src/cmds/ruby/rake_cmd.rs` | 🟡 `ig:filters/ruby.toml`, `pest.toml`, `phpunit.toml`, `rspec.toml` | Add minitest STATE parser |
| `rspec` | `rtk:src/cmds/ruby/rspec_cmd.rs` | 🟡 (TOML) | Add Rust JSON-injection parser |
| `rubocop` | `rtk:src/cmds/ruby/rubocop_cmd.rs` | ❌ | New (with autocorrect-mode opt-out) |
| `bundle install/update` | `rtk:src/filters/bundle-install.toml` | ✅ acceptable as TOML |  |
| `dotnet build/test/format` (+trx/binlog) | `rtk:src/cmds/dotnet/*.rs` | 🟡 `ig:filters/dotnet.toml` | Add Rust parsers later (trx XML, binlog) — low pri |
| `aws <svc> <verb>` (sts, ec2, lambda, s3, dynamodb, iam, cloudformation, logs) | `rtk:src/cmds/cloud/aws_cmd.rs` | 🟡 `ig:filters/aws-extended.toml`, `cloud.toml` | TOML covers most; add dedicated `ig aws` subcommand for sts/ec2/lambda/dynamodb/iam/logs |
| `docker/kubectl/compose` | `rtk:src/cmds/cloud/container.rs` | 🟡 `ig:src/cmds/docker.rs` + `kubectl.toml` + `docker.toml` | Extend `docker.rs`, add `kubectl` Rust path with TREE format |
| `psql/curl/wget` | `rtk:src/cmds/cloud/{psql_cmd,curl_cmd,wget_cmd}.rs` | 🟡 TOML for `net-tools.toml`, no `psql` | Add `ig psql`, `ig curl --tee`, `ig wget` |
| `ls` | `rtk:src/cmds/system/ls.rs` | ✅ `ig:src/ls.rs` |  |
| `tree` | `rtk:src/cmds/system/tree.rs` | ❌ | Add `ig tree` (alias of `ig ls --tree`?) |
| `read` (CODE filter) | `rtk:src/cmds/system/read.rs` | ✅ `ig:src/read.rs` with `-s/-a/-b/-r/-d/-p` (richer than RTK!) | Already better; align CLI flag names (`-l <level>` from RTK is alternative) |
| `find` | `rtk:src/cmds/system/find_cmd.rs` | 🟡 `ig:src/cli.rs` exposes `ig files` (basically equivalent) | Add an `ig find` alias for muscle memory |
| `grep` | `rtk:src/cmds/system/grep_cmd.rs` | ✅ `ig` itself IS grep — **with a trigram index** (game-changer) | Reuse the index for `ig run grep …` rewrites |
| `json` | `rtk:src/cmds/system/json_cmd.rs` | ✅ `ig:src/cmds/json_cmd.rs` |  |
| `env` | `rtk:src/cmds/system/env_cmd.rs` | ✅ `ig:src/cmds/env_cmd.rs` |  |
| `deps` | `rtk:src/cmds/system/deps.rs` | ✅ `ig:src/cmds/deps.rs` |  |
| `log <file>` (DEDUP) | `rtk:src/cmds/system/log_cmd.rs` | ❌ | Add `ig log` |
| `summary <cmd>` | `rtk:src/cmds/system/summary.rs` | ❌ | Add `ig summary` |
| `wc` | `rtk:src/cmds/system/wc_cmd.rs` | ❌ | TOML or tiny Rust mod |
| `proxy <cmd>` / `run <cmd>` | RTK main fallback | ✅ `ig:src/cmds/run.rs` + `ig:src/runner.rs` |  |
| `test <cmd>` (generic) | RTK runner | ✅ `ig:src/cmds/test_runner.rs` |  |
| `err <cmd>` | RTK runner | ✅ `ig:src/cmds/err.rs` |  |
| `init` (per-agent) | `rtk:src/hooks/init.rs` | 🟡 `ig:src/setup.rs` (2,558 LOC, covers claude/codex/cursor/copilot/gemini/opencode/windsurf/cline) | Add hermes / kilocode / antigravity + JSON patch flow for cursor/gemini hooks; introduce `--agent` selector + `--show/--uninstall/--dry-run/--hook-only/--auto-patch` flags |
| `hook` sub-enum + `rewrite` exit-code protocol | `rtk:src/hooks/{rewrite_cmd,permissions}.rs` | 🟡 `ig:src/rewrite.rs` (`Commands::Rewrite` exists) | Add Allow/Ask/Deny/Default permission verdicts and exit-code protocol (0/3/2/3) |
| `hook-audit` | `rtk:src/hooks/hook_audit_cmd.rs` | ❌ | New |
| `verify` (TOML inline tests) | `rtk:src/hooks/verify_cmd.rs` | ✅ `Commands::Verify` already in CLI; verify implementation exists |  |
| `discover` | `rtk:src/discover/*` | ✅ `ig:src/discover.rs` (700 LOC) | Align: add RTK_DISABLED bucket, `--format json`, agent-integration-status block |
| `learn` | `rtk:src/learn/*` | 🟡 `Commands::Learn` exists | Verify detector parity |
| `session` | `rtk:src/analytics/session_cmd.rs` | ✅ `Commands::Session` |  |
| `gain` + sub-modes | `rtk:src/analytics/gain.rs` | ✅ `ig:src/gain.rs` (1k+ LOC, very complete: `--graph --history --daily --weekly --monthly --json --discover --missed --compare --quota`) | **Better than RTK** already; gap is the backing store (JSONL vs SQLite — see §4) |
| `cc-economics` / `ccusage` / `economics` | `rtk:src/analytics/{cc_economics,ccusage}.rs` | 🟡 `Commands::Economics` | Verify ROI formula and per-tier pricing |
| `telemetry {enable/disable/status/forget}` | `rtk:src/core/{telemetry,telemetry_cmd}.rs` | ❌ | **Decision needed**: ship telemetry off-by-default (RGPD-compliant), or skip entirely (ig is OSS+local and may want zero phone-home as a feature). Spec proposes opt-in stub. |
| Built-in TOML filter coverage | 60 `*.toml` in RTK | ✅ 43 in `ig:filters/` | Audit gaps: add missing tools (`jj`, `mise`, `jq`, `gradle`, `liquibase`, `markdownlint`, `mvn-build`, `nx`, `ollama`, `oxlint`, `ping`, `pio-run`, `pre-commit`, `ps`, `quarto-render`, `rsync`, `shellcheck`, `shopify-theme`, `skopeo`, `sops`, `spring-boot`, `ssh`, `stat`, `swift-build`, `systemctl-status`, `task`, `tofu-*`, `trunk-build`, `xcodebuild`, `yamllint`, `yadm`, `fail2ban-client`, `df`, `du`, `basedpyright`, `composer-install`, `ansible-playbook`, `gcc`, `gcloud`, `iptables`, `hadolint`, `helm`, `jira`, `just`, `mix-*`) |
| Project-local `.rtk/filters.toml` (single-file format) | RTK | 🟡 ig uses **directory** `.ig/filters/*.toml` (multi-file, trust-gated) | ig's design is arguably better; keep, but document the divergence and add an importer for users coming from RTK |
| Sentinel-wrapped markdown blocks | `rtk:src/hooks/init.rs:66-67` | ✅ already in `ig:src/setup.rs` (per CLAUDE.md, v1.19.1+ managed-block) |  |
| Permission deny/ask rules | `rtk:src/hooks/permissions.rs` | ❌ | New — small but security-critical |
| Hook script SHA-256 integrity | `rtk:src/hooks/integrity.rs` | ❌ | New |
| Hook drift warning (`hook_check`) | `rtk:src/hooks/hook_check.rs` | ❌ | New |
| Ultra-compact mode (`-u`) | global flag | ❌ | New global flag |
| Verbosity levels (`-v/-vv/-vvv`) | global flag | 🟡 `ig` has some flags but no unified `Count` action | Unify under `verbose: u8` |
| Tee | `rtk:src/core/tee.rs` | ✅ `ig:src/tee.rs` (with `Commands::Tee {show,list,clear}`) |  |
| `extract_json_object` (robust JSON extraction after `dotenv`/`pnpm` prefixes) | `rtk:src/parser/mod.rs` | ❌ | Add helper |
| `OutputParser` trait + `TestResult/TestFailure/TokenFormatter/FormatMode` shared types | `rtk:src/parser/{mod,types,formatter}.rs` | ❌ | New, foundational for all test-runner Rust filters |
| Hermes / OpenClaw plugins | `rtk:hooks/hermes/`, `rtk:openclaw/` | ❌ | Future; not v2.x blocker |

---

## 3. Plan PR-by-PR (≤ 6 mergeable PRs)

Each PR is sized to land in 1–2 weeks, with tests, and to keep `main` green at every step. The order respects dependencies (parser trait → consumers; permission engine → integrity → init refactor; tracking migration → analytics).

### PR 1 — Foundation: parser trait, ultra-compact flag, verbosity unification, tracking → SQLite

**Effort: M**

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/parser/mod.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/parser/types.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/parser/formatter.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/analytics/sqlite.rs` (new)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/analytics/migrate.rs` (JSONL→SQLite one-shot)

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (add `--ultra-compact/-u`, `verbose: u8` via `ArgAction::Count`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/tracking.rs` (write to both JSONL **and** SQLite during a transition window, then read-only from SQLite in PR 4)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/Cargo.toml` (add `rusqlite = { version = "0.31", features = ["bundled"] }`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/main.rs` (wire global flags into a `RunOptions` ctx struct)

**Tests / goldens:**
- `tests/parser_trait.rs` — `TestResult::Full/Partial/Failed` round-trip
- `tests/tracking_sqlite.rs` — `record() → get_summary()` round-trip, 90-day cleanup
- `tests/migrate_jsonl_to_sqlite.rs` — golden JSONL file with 50 entries imports without loss; idempotent on re-run

**Risks:**
- `rusqlite` `bundled` flag pulls in SQLite C source → +400 KB binary. Acceptable for the parity gain.
- macOS `~/Library/Application Support/ig/` vs Linux `~/.local/share/ig/` — copy RTK's `super::constants::RTK_DATA_DIR` pattern.
- Double-write window risk: spec **mandates** that PR 1 ships only the dual-write; PR 4 cuts JSONL out. A re-entrant `flock` on the SQLite WAL file replaces the current `libc::flock` on the JSONL.

### PR 2 — Permission engine, hook integrity, hook drift warning, rewrite exit-code protocol

**Effort: M**

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/hooks/permissions.rs` (port `rtk:src/hooks/permissions.rs`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/hooks/integrity.rs` (SHA-256 of installed hook script)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/hooks/hook_check.rs` (1/day drift warning)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/hooks/audit.rs` (new `ig hook-audit`)

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/rewrite.rs` (apply `check_command` before rewriting; switch exit codes to `0/1/2/3` protocol; current `run_rewrite` returns `()` — change to `i32`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (`Commands::HookAudit { since: u32 }`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/hooks/mod.rs` (re-export)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/hooks/claude/rtk-rewrite.sh` (existing? check; align exit-code handling)

**Tests / goldens:**
- `tests/permissions_exit_codes.rs` — table-test each `(cmd, verdict)→exit` mapping; **MUST include** the `Default → 3` sentinel from RTK issue #1155
- `tests/permissions_deny_rules.rs` — `rm -rf /`, `git reset --hard`, `git clean -fd` all → Deny
- `tests/permissions_ask_rules.rs` — `git push --force [origin main]` → Ask
- `tests/integrity_drift.rs` — flip a byte in the installed hook, expect warning on next run, rate-limited to 1/day

**Risks:**
- The deny list is security-critical. **Allowlist policy**: only ig-supplied rules ship by default; user/project rules go through `~/.config/ig/permissions.toml` (parsed but only `deny` and `ask` honored; no allow-by-config to prevent privilege creep).
- Re-keying exit codes is a breaking change for users with custom hook scripts. Ship a `IG_HOOK_EXIT_LEGACY=1` env var as a one-release escape hatch + a `CHANGELOG.md` callout.

### PR 3 — `init` refactor: per-agent installer, all 11 agents, `--show/--uninstall/--dry-run/--hook-only/--auto-patch`

**Effort: L**

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/setup.rs` (2,558 LOC → split into `setup/{mod,claude,codex,cursor,copilot,gemini,opencode,hermes,windsurf,cline,kilocode,antigravity}.rs`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (`Commands::Setup` → add `--agent`, `--hook-only`, `--auto-patch`, `--no-patch`, `--show`, `--uninstall`; or alias as `Commands::Init` for RTK muscle memory)

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/setup/agents/{hermes.rs,kilocode.rs,antigravity.rs,opencode_plugin.rs}`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/hooks/{hermes,kilocode,antigravity}/{ig-awareness.md,README.md}` (mirrors `rtk:hooks/*/`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/hooks/opencode/ig.ts` (TS plugin)

**Tests / goldens:**
- `tests/setup_dry_run_claude.rs` — `ig setup --agent claude --dry-run` matches golden output (no files touched)
- `tests/setup_idempotent.rs` — run twice, second run reports "already up to date" for every agent
- `tests/setup_show.rs` — `ig setup --show` after a clean install enumerates active hooks
- `tests/setup_uninstall_per_agent.rs` — `--uninstall --agent claude` removes claude-only artifacts and leaves codex untouched

**Risks:**
- The `~/.claude/settings.json` patch is the touchiest operation: it must preserve unrelated keys + ordering. Use `serde_json::Value` with `preserve_order` feature and only touch the `hooks.PreToolUse` array — never rewrite the whole file. Same for Cursor `hooks.json` and Gemini `settings.json`.
- Hermes plugin is a Python file; ship as `include_str!` and write to `~/.hermes/plugins/ig-rewrite/__init__.py`. Best-effort detection — skip silently if `~/.hermes/` is absent.
- Cross-platform path canonicalisation: `dirs::config_dir()` differs on macOS (`~/Library/Application Support`) vs Linux (`~/.config`). Use the existing `ig:src/cache.rs::ensure_layout()` pattern.

### PR 4 — Rust filter modules for the high-value test runners + linters

**Effort: L**

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/test/{mod,vitest,jest,playwright,pytest,cargo_test,go_test,rspec,rake}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/lint/{mod,eslint,biome,tsc,prettier,ruff,mypy,rubocop,golangci}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/build/{mod,next,prisma,cargo_build}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/pkg/{mod,pnpm,npm,pip}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/util/package_manager.rs` (detect pnpm-lock / yarn.lock / fallback npx)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/util/extract_json.rs` (robust JSON object extraction after stdout noise)

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (add `Commands::{Vitest, Jest, Playwright, Pytest, Pnpm, Tsc, Lint, Prettier, Next, Prisma, Ruff, Mypy, Pip, Rubocop, GolangciLint}` — each `#[arg(trailing_var_arg, allow_hyphen_values)]`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/main.rs` (route)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/mod.rs` (re-export submodules)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/test_runner.rs` (router becomes "if dedicated parser exists, use it; else generic")

**Tests / goldens:**
- `tests/fixtures/{vitest,jest,playwright,pytest,go_test,rspec,golangci}/` — `raw.txt` + `expected.compact.txt` + `expected.ultra.txt` per tool, mirroring `rtk:tests/fixtures/`
- One golden per parser fallback path: vitest with `dotenv` prefix → still parses
- `tests/test_pytest_xfail.rs` — XFAIL/XPASS surfacing
- `tests/test_go_test_ndjson.rs` — interleaved package events

**Risks:**
- **Parsing fragility**: tool output formats drift between minor versions. Mitigation: every parser MUST have a TOML/regex fallback (`extract_json_object` then `regex` then passthrough+warning). RTK already does this — copy the pattern.
- **ANSI**: every parser strips ANSI first via shared util. UTF-8: never assume; use `String::from_utf8_lossy`.
- **Exit code preservation**: tests must check both exit code AND filtered content per RTK's pattern (`rtk:docs/contributing/ARCHITECTURE.md:830-854`).
- **Recursive proxy loop**: `route_to_dedicated` in `ig:src/cmds/run.rs` already guards against `cargo → ig run cargo → ig cargo → …`. Extend the `DEDICATED` allowlist to include new subcommands.

### PR 5 — `gh/glab/gt` + cloud (`aws`, `kubectl`, `psql`, `curl`, `wget`, `log`, `summary`)

**Effort: M**

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/git/{gh,glab,gt}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/cloud/{aws,kubectl,psql,curl,wget}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/system/{log,summary,tree,wc}.rs`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/util/markdown.rs` (port `gh_cmd::filter_markdown_body` — HTML comments, badges, image-only lines, hrules, multi-blank collapse, code-fence preservation)

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (`Commands::{Gh,Glab,Gt,Aws,Kubectl,Psql,Curl,Wget,Log,Summary,Tree,Wc}`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cmds/docker.rs` (extend with `compose`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/rewrite.rs` (add rewrite mappings for the new subjects so the hook picks them up)

**Tests / goldens:**
- `tests/fixtures/gh/{pr_list,pr_view_42,issue_list,run_list}/` per RTK fixture
- `tests/fixtures/aws/{sts_get_caller_identity,ec2_describe_instances,lambda_list_functions,dynamodb_scan,iam_list_roles}/`
- `tests/fixtures/kubectl/{get_pods,logs}/`
- `tests/fixtures/curl_tee.rs` — curl with body >1MiB → output truncated + tee path printed

**Risks:**
- AWS CLI output volume is enormous and version-dependent. **Strategy**: per-service parser, pure JSON only (we always pass `--output json` via the dispatcher when we own the args), fall back to passthrough on unknown verbs.
- `kubectl` output drifts wildly across server versions; restrict the Rust path to `get/logs/describe/apply` and TOML-filter the rest.
- `curl` interactivity (`-N` no-buffer, TTY progress) — strict opt-in: only auto-tee when stdin is not a TTY AND stdout is captured.

### PR 6 — `discover` parity, `learn` polish, optional opt-in telemetry stub, RTK-compat TOML import, ultra-compact mode wiring, docs

**Effort: M**

**Files modified:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/discover.rs` (add RTK_DISABLED bucket, `--format json`, agent-integration-status block, `--all` to scan all projects)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/cli.rs` (extend `Commands::Discover`)
- All cmd modules touched in PRs 4–5: wire `-u` to compact output paths

**Files created:**
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/telemetry/{mod,consent,ping,fields}.rs` (opt-in, off by default, no URL compiled in unless `IG_TELEMETRY_URL` env var at build time — same pattern as `rtk:src/core/telemetry.rs`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/src/setup/import_rtk.rs` (one-shot: read `~/.config/rtk/filters.toml` and `.rtk/filters.toml`, emit equivalent `~/.config/ig/filters/imported-from-rtk.toml` and `.ig/filters/imported-from-rtk.toml`; auto-trust only project-local file if user runs `ig setup --import-rtk`)
- `/Users/kevin/Documents/lab/sandbox/instant-grep/docs/MIGRATING_FROM_RTK.md`
- `/Users/kevin/Documents/lab/sandbox/instant-grep/CHANGELOG.md` (v2.0.0 entry)

**Tests / goldens:**
- `tests/discover_format_json.rs` — schema-stable JSON output
- `tests/telemetry_default_off.rs` — no payload sent unless `consent_given == Some(true) && enabled`
- `tests/telemetry_env_kill.rs` — `IG_TELEMETRY_DISABLED=1` shortcircuits even with consent
- `tests/import_rtk_round_trip.rs` — RTK's project-local `.rtk/filters.toml` → equivalent ig output for a fixture

**Risks:**
- Telemetry: legal/PR exposure. Default position recommended in this spec: **do NOT compile any URL into the open-source binary**. Ship the consent flow and the field collector, but only "active" telemetry if the user does `cargo install --features telemetry-url` with `IG_TELEMETRY_URL`. This keeps ig privacy-safe by default while preserving the architecture if a hosted service ever appears.
- RTK importer: trust model — never auto-trust the *user* RTK filter file, only the project-local one (and only if the project is a git repo at HEAD, to keep the trust scope auditable).

---

## 4. Architecture

### 4.1 Module layout (post-PR-6)

```
src/
├── cli.rs                                # clap enum (RTK-shaped subcommands added)
├── main.rs                               # router; global flags → RunOptions
├── analytics/
│   ├── sqlite.rs                         # rusqlite-backed tracking + queries
│   ├── migrate.rs                        # JSONL → SQLite one-shot
│   ├── gain.rs                           # (existing) reads SQLite now
│   └── economics.rs                      # (existing)
├── parser/
│   ├── mod.rs                            # OutputParser trait
│   ├── types.rs                          # TestResult, TestFailure, FormatMode, TokenFormatter
│   └── formatter.rs                      # shared formatters (compact / ultra-compact)
├── cmds/
│   ├── git/{git.rs, gh.rs, glab.rs, gt.rs, diff_cmd.rs}
│   ├── test/{mod.rs, vitest.rs, jest.rs, playwright.rs, pytest.rs, cargo_test.rs, go_test.rs, rspec.rs, rake.rs}
│   ├── lint/{mod.rs, eslint.rs, biome.rs, tsc.rs, prettier.rs, ruff.rs, mypy.rs, rubocop.rs, golangci.rs}
│   ├── build/{mod.rs, next.rs, prisma.rs, cargo_build.rs}
│   ├── pkg/{mod.rs, pnpm.rs, npm.rs, pip.rs}
│   ├── cloud/{aws.rs, kubectl.rs, psql.rs, curl.rs, wget.rs}
│   ├── system/{log.rs, summary.rs, tree.rs, wc.rs, json_cmd.rs, env_cmd.rs, deps.rs}
│   ├── run.rs / test_runner.rs / err.rs   # generic wrappers (existing)
│   └── util/{package_manager.rs, extract_json.rs, markdown.rs, ansi.rs}
├── hooks/
│   ├── mod.rs
│   ├── permissions.rs                    # Allow/Ask/Deny/Default
│   ├── integrity.rs                      # SHA-256 verify of installed hooks
│   ├── hook_check.rs                     # 1/day drift warning
│   ├── audit.rs                          # `ig hook-audit`
│   └── copilot.rs                        # (existing, slim)
├── setup/
│   ├── mod.rs
│   └── agents/{claude.rs, codex.rs, cursor.rs, copilot.rs, gemini.rs, opencode.rs, windsurf.rs, cline.rs, hermes.rs, kilocode.rs, antigravity.rs}
├── telemetry/
│   ├── mod.rs                            # gated on IG_TELEMETRY_URL build env
│   ├── consent.rs                        # CLI flow
│   ├── ping.rs                           # 23h marker, 2s timeout, fire-and-forget thread
│   └── fields.rs                         # payload assembly
├── filter/{loader.rs, pipeline.rs, mod.rs}  # (existing TOML DSL; unchanged)
├── tracking.rs                           # facade — delegates to analytics::sqlite
├── rewrite.rs                            # (existing) + permissions integration
└── ... (existing: index/, query/, semantic/, etc.)
```

### 4.2 SQLite vs JSONL — pros/cons and the call

ig today (`ig:src/tracking.rs`) writes JSONL with `flock` and `ig:src/gain.rs` (1k+ LOC) does in-memory aggregation. RTK writes SQLite from day one.

| Aspect | JSONL (today) | SQLite (RTK) |
|---|---|---|
| Single-file portability | great (one `~/.local/share/ig/history.jsonl`) | great (one `tracking.db`) |
| Append-only, crash-safe | yes (flock + O_APPEND) | yes (WAL mode) |
| Aggregation cost | O(N) full scan per `ig gain` | O(log N) via indices |
| Concurrent writers | OK via `flock` | better — WAL handles MVCC |
| Schema evolution | painful (must re-parse all rows) | `ALTER TABLE` |
| Binary size | +0 KB | +400 KB with `bundled` |
| Project-scoped queries | grep+parse | `WHERE project_path GLOB ?` |
| Eyeball-debuggable | yes (`tail history.jsonl`) | only with `sqlite3` CLI |
| RTK feature parity (`gain --daily/weekly/monthly/json`, `cc-economics`) | doable but slow at >100k rows | native |
| Telemetry quality metrics (passthrough_top, parse_failures_24h, low_savings_commands) | re-parse the world | one indexed query |

**Recommendation**: migrate to SQLite (PR 1) but keep the JSONL writer behind `IG_TRACKING_JSONL=1` for one minor release for users with shell scripts that `jq` the JSONL. Provide `ig migrate-history` as a one-shot importer. `ig:src/gain.rs` keeps its public function signatures; the data layer swaps underneath.

### 4.3 Does the TOML pipeline suffice, or do we need a `Strategy` trait?

Today the TOML pipeline (`ig:src/filter/pipeline.rs`) covers: strip_ansi, regex replace, keep/drop lines, truncate per line, head, tail, max_lines, on_empty, dedup_consecutive. That handles **TOML strategies** (one of RTK's 12) cleanly.

The other 11 RTK strategies (STATE machine, NDJSON streaming, JSON-with-fallback, STRUCT extraction, GROUP-by-pattern, FAIL-only, TREE compression, PROG-stripping, CODE-aware, JSONTXT dual, DEDUP) require **executable logic**. RTK encodes them as one Rust module per command, with a small shared `parser::OutputParser` trait (`rtk:src/parser/mod.rs`) used only by test runners.

**Proposal**: introduce a `trait OutputParser { type Output; fn parse(&str) -> ParseResult<Self::Output>; }` in `ig:src/parser/mod.rs` (PR 1), with shared types `TestResult`, `TestFailure`, `TokenFormatter`, `FormatMode { Compact, Ultra }`. Apply it where it helps (test runners, linters). For one-off commands (`gh pr view`, `aws sts`), keep one-off functions — over-abstracting yields churn without value (RTK confirms this: only test runners share the trait).

The TOML DSL stays as the **catch-all/fallback** and as the user-extensibility surface. Built-ins that are pure regex shapes (`bundle-install`, `helm`, `terraform-plan`) stay in TOML.

### 4.4 Using the trigram index to differentiate

ig already owns a fast trigram index (`ig:src/index/*.rs`). Three opportunities the daemon-less v2 makes natural:

1. **`ig grep` / `ig find` rewrites** — when the hook sees `grep -r foo .` or `find . -name '*.rs'`, rewrite to `ig grep` / `ig files`. Already partially done (`ig:src/rewrite.rs::rewrite_grep/rewrite_find/rewrite_rg`). RTK has no equivalent — its `rtk grep`/`rtk find` are linear scans. Sell this as a 100× speedup on warm repos AND token compression in one call.
2. **`ig read --signatures` boosted by the index** — when `-r <pattern>` is provided to bias entropy scoring, query the trigram index for `<pattern>` first to seed the "important lines" set in O(ms) rather than scanning. (`ig:src/read.rs` already supports `-r/-b`; extend to consult the index for files >50 KB.)
3. **`ig discover --semantic`** — once `--semantic` (PMI synonym expansion, `ig:src/cli.rs:91-96`) is mainstreamed, `ig discover` could expand the "supported by ig" detection to recognise semantically equivalent invocations (e.g. `cat <file> | grep <pat>` → `ig grep <pat> <file>`). RTK has no semantic layer.

### 4.5 Filter trust model

ig keeps `.ig/filters/*.toml` (multi-file, trust-gated via `ig:src/trust.rs`) instead of RTK's single `.rtk/filters.toml`. This is **strictly safer**: each file is hashed at trust-time, and `ig trust <file>` records the hash in `~/.config/ig/trusted-filters.json`. Mutation invalidates trust. RTK's model trusts the whole project file implicitly.

Keep the divergence. Document in `docs/MIGRATING_FROM_RTK.md`. Add `ig setup --import-rtk` to flatten RTK's single-file format into the per-file layout, and only auto-trust if the project is at a clean HEAD.

---

## 5. CLI spec — new and changed commands

Conventions: square brackets are optional; `<...>` is a value; `--ultra/-u` and `-v/-vv/-vvv` are global; every test/lint/build subcommand uses `#[arg(trailing_var_arg = true, allow_hyphen_values = true)]` so the wrapped tool's args pass through unchanged.

### 5.1 New globals

```
-u, --ultra-compact         ASCII icons + single-line summaries (max compression)
-v                          verbose: -v debug, -vv exec line, -vvv raw output
--skip-env                  do not preserve env-prefix on rewrite (escape hatch)
--explain                   print why the filter decided what it kept/dropped (NEW, ig-only)
```

**`--explain`** is an ig-unique differentiator (rtk has nothing equivalent). Output to stderr: every stage of the pipeline (TOML or Rust) emits a 1-line diagnostic — match decision, lines kept, lines dropped, fallback triggered. Format `[ig:explain] <stage>: <action> (<count>)`. Implementation: thread a `Diagnostics` collector through `runner::run_filtered` and Rust parser modules.

### 5.2 `ig init` (renamed/aliased from `ig setup`)

```
ig init [--agent <CLAUDE|CODEX|CURSOR|COPILOT|GEMINI|OPENCODE|WINDSURF|CLINE|HERMES|KILOCODE|ANTIGRAVITY>]
        [-g, --global] [--hook-only] [--ig-md|--awareness-only]
        [--auto-patch | --no-patch] [--show | --uninstall] [--dry-run] [--quiet]
        [--import-rtk]    # ig-only: convert ~/.config/rtk/filters.toml
```

Behaviour:
- No `--agent` → auto-detect every installed agent (current `ig setup` behaviour, kept).
- `--show` → enumerate active hooks and their SHA-256 status (OK / drifted / missing).
- `--uninstall` → remove hook scripts, settings.json entries, awareness markdown for the given agent (or all).
- Exit 0 on every success / no-change; 1 on any IO error; 2 on permission-denied (e.g. cannot write `~/.claude/settings.json`).

Golden tests: `tests/setup_*.rs` (PR 3).

### 5.3 `ig hook-audit`

```
ig hook-audit [--since N=7] [--format text|json]
```

Scans Claude Code session JSONL files (and codex/gemini equivalents) for hook events in the last N days. Reports: total events, hook-rewrite invocations, deny/ask verdicts hit, exec failures, average rewrite latency (parsed from session timing fields when present). Mirrors `rtk hook-audit`.

### 5.4 `ig discover` (extended)

```
ig discover [--project <path> | --all] [--since N=30] [--limit N=15]
            [--format text|json] [--shell]
```

Adds vs today:
- `--all` switch
- `--format json` (schema-stable)
- agent-integration-status block in text output
- RTK_DISABLED bucket (parity with RTK)

### 5.5 New per-tool subcommands (all wrap the tool, all take `trailing_var_arg`)

```
ig gh <args...>          # PR/issue/run/repo/api/release, markdown-noise stripped
ig glab <args...>        # mirror of gh
ig gt <args...>          # Graphite CLI
ig vitest <args...>      # JSON parse, failures-only
ig jest <args...>        # same
ig playwright <args...>
ig pytest <args...>      # injects --tb=short -q -rxX, state-machine
ig pnpm <verb> <args...> # exec/install/list/outdated/run/run-script
ig npm <verb> <args...>
ig npx <cmd> <args...>
ig tsc <args...>         # group by file then error code
ig lint [eslint|biome] <args...>
ig prettier <args...>    # files needing fmt only
ig next <args...>        # next build
ig prisma <args...>      # strip ASCII art
ig ruff <verb> <args...> # check (JSON) / format (text)
ig mypy <args...>
ig pip <verb> <args...>  # auto-detect uv
ig rubocop <args...>     # skip JSON inject in -a/-A modes
ig golangci-lint <args...>
ig aws <service> <verb> <args...>
ig kubectl <verb> <args...>
ig psql <args...>
ig curl <args...>        # auto-tee body when capturing
ig wget <args...>        # strip progress
ig log <file>            # DEDUP consecutive lines with counts
ig summary <cmd...>      # heuristic
ig tree [path]
ig wc <file>
```

### 5.6 Telemetry (gated on build feature)

```
ig telemetry status      # show consent state, last ping, payload preview
ig telemetry enable      # interactive consent (writes ~/.config/ig/config.toml)
ig telemetry disable     # withdraw consent
ig telemetry forget      # disable + delete device salt + best-effort server-erasure
```

Default: completely no-op unless built with `--features telemetry-url IG_TELEMETRY_URL=...`. `IG_TELEMETRY_DISABLED=1` short-circuits even with consent.

### 5.7 Exit codes (full table, all subcommands)

| Code | Meaning |
|---|---|
| 0 | success / no-change |
| 1 | ig internal error (parse, IO, filter panic) |
| 2 | permission denied OR deny-rule matched (rewrite path) |
| 3 | ask-rule matched (rewrite path) |
| N | preserved exit code from the wrapped tool when N ∉ {1,2,3} |

The N≥4 carve-out matches RTK's behaviour: `git` returns 128 for "not a repo", `lint` returns 1 on findings, etc. The collision between "ig error" and "tool exit 1" is unavoidable; mitigate by always logging to stderr when the failure is ig-internal.

---

## 6. Pitfalls

1. **Daemon-removed regressions**. Watcher rebuilds, `ig hold begin/end`, `~/.cache/ig/daemon/*` are gone (commit `f716f85`). Every code path that referenced `~/.cache/ig/daemon/` must be confirmed dead. Search target: `daemon.sock`, `seal.bin`, `TenantState`, `GlobalState`, `session_begin`, `session_end`. Anything that still calls these will compile but no-op or panic at runtime.
2. **Hook-rewrite security**. `ig rewrite` is invoked on every Bash tool call by the agent. A misbehaving regex that rewrites `rm -rf /tmp/foo` into something destructive is a worst-case bug. Mitigations: (a) allowlist of safe rewrites only; (b) `permissions::check_command` runs BEFORE rewrite; (c) RTK's `Default → 3 (ask)` invariant is REQUIRED so unknown commands always prompt; (d) `--dry-run` flag on the hook script for debugging; (e) golden tests for every deny pattern.
3. **Cross-platform**. macOS uses `~/Library/Application Support/ig/tracking.db`, Linux `~/.local/share/ig/tracking.db`. Settings paths likewise. Windows is currently second-class for ig — match RTK's stance (filters work, hook does not; fall back to awareness-md injection).
4. **ANSI**. Always strip BEFORE regex matching (RTK's pattern). Some tools emit OSC-8 hyperlinks and bracketed-paste sequences — extend `util::ansi::strip()` accordingly. Save the raw (with ANSI) to tee so the user can re-view colored output.
5. **UTF-8 safety**. `String::from_utf8_lossy(&output.stdout)` everywhere. Never use byte slicing on lossy strings unless you re-tokenize.
6. **Exit code preservation**. The single biggest regression risk: tests/CI relies on the wrapped tool's exit code propagating. Every `Command::output()?` site needs a corresponding `std::process::exit(output.status.code().unwrap_or(1))` if filter returns Ok. Already done in `ig:src/runner.rs:19` — verify on every new module.
7. **Compat ascendante on `.ig/filters/*.toml` trust**. After PR 1's tracking migration, the trust DB (`~/.config/ig/trusted-filters.json`) is untouched. After PR 6's RTK importer, the imported file is a **new** path, so it's untrusted by default — never auto-trust unless the project is at a clean git HEAD AND `--import-rtk` was explicitly requested.
8. **Telemetry compliance**. Even with consent + opt-in, NEVER collect command arguments, file paths, secrets. Top-commands payload is **tool name only** (first token). Salted SHA-256 device hash is the only stable identifier. Provide `--dry-run` on `ig telemetry status` that prints exactly what would be sent.
9. **SQLite WAL on macOS**. WAL files (`*.db-wal`, `*.db-shm`) are sensitive to iCloud Drive sync. If `~/Library/Application Support/ig/` ever lands under an iCloud-synced path, append `.nosync` markers or fall back to the XDG cache. Test on a fresh macOS account.
10. **Hook idempotence**. Re-running `ig init --agent claude` must converge: same hash, no duplicate entries in `settings.json`, no flapping awareness-md. Use ig's existing sentinel block (`<!-- ig-instructions -->`) plus a settings.json *patch-by-key* approach (touch only `hooks.PreToolUse[].hooks[where command contains "ig"]`).
11. **Filter regex catastrophic backtracking**. Every user-supplied regex from TOML and from permissions config must be size-bounded. Use `regex::Regex::with_size_limit` (default 10 MB compiled) and refuse to compile filters exceeding 1k chars. Log + skip the bad filter rather than aborting startup.
12. **`pnpm exec --` injection**. When auto-detecting `pnpm`, never blindly forward args to `pnpm exec` — RTK escaped a CVE (#1234-style) by always splitting on the `--` separator. Mirror that in `cmds/util/package_manager.rs`.
13. **Tee directory growth**. `.ig/tee/` (or central `~/.cache/ig/tee/`) needs rotation. ig already has `ig tee clear` and `ig gc` handles cache pruning — extend `ig gc` to also prune tee entries older than 30 days. RTK has `[tee] max_files = N`, `max_size_mb = N` in `config.toml` — port this.
14. **`ig rewrite` recursion**. Hook rewrites `cargo test` → `ig cargo test`. If a buggy filter then exec's `cargo test` *again*, the hook fires *again*. Guard: in `permissions::check_command` strip `ig ` prefix before classifying; only rewrite commands that don't already start with `ig`.

---

## 7. How `ig` ends up *better* than RTK

1. **Trigram index for `grep/find/read`** — RTK's `rtk grep` is a linear scan; `ig grep` is sub-ms on a warm repo. This is a free win once the rewrite engine maps `grep -r/-rn` to `ig` (already partially in `ig:src/rewrite.rs`). Sell it as "compression + speed", not just compression.
2. **`--explain` flag** — RTK has nothing equivalent. Make filter decisions observable so agents (and humans) can debug "why did my output get truncated". Cost: ~50 LOC per stage of the pipeline.
3. **`--semantic` synonym expansion** — `ig:src/cli.rs:91-96` mentions PMI co-occurrence. RTK has no semantic layer. Apply to `ig grep`, `ig discover`, and `ig read -r <pattern>` for richer context selection.
4. **Per-file `.ig/filters/*.toml` with hash-pinned trust** — strictly safer than RTK's single-file model.
5. **Tee + `ig gc` unified storage** — RTK has tee but separate manual cleanup. ig has `ig gc` doing automatic pruning; extend it to tee.
6. **No daemon = no mode-mode-mode debugging** — RTK never had a daemon. ig just shed one. Communicate this in `docs/MIGRATING_FROM_DAEMON.md` so existing v1.x users understand the contract (no more `ig hold begin/end`, watcher rebuilds, etc.) and so future RTK migrants see "we ship the same lean model".
7. **`ig pack` / `ig smart` / `ig symbols` / `ig context`** — RTK has none of these. They are agent-context generators that pair perfectly with the rewrite engine: when the agent runs `cat <file>`, rewrite to `ig read -s` for signatures-only; when the agent runs `ls -R`, rewrite to `ig pack` if `<file count> > 50`.
8. **`ig autoignore`** — `ig:src/autoignore.rs` generates project-tailored `.ignore`. RTK has nothing here. Cross-sell with `ig init`: at install, optionally write `.ignore` matched to the detected stack.
9. **`ig run` routes to dedicated subcommand** — `ig:src/cmds/run.rs::route_to_dedicated` already routes `ls`, `git`, `files`, `read`, `smart` to dedicated paths. Extend the allowlist to include every new RTK-parity subcommand from PR 4–5 so users don't have to learn `ig pnpm` vs `ig run pnpm` — they get the dedicated parser regardless.
10. **Cargo features for telemetry** — RTK compiles the URL in (with a default that's a no-op if env vars are missing). ig should make telemetry compile-time *opt-in* via cargo feature, so the OSS binary is provably hermetic.
11. **`ig discover --semantic`** (future) — once the PMI vocabulary is per-project, `discover` can recognise patterns RTK misses: a project that exclusively uses `mise exec -- pnpm test` looks Unsupported to RTK; ig can learn the alias from session traces.

Other opportunities surfaced while reading RTK code:

- RTK's `parser::OutputParser` returns a `ParseResult::Full | Partial | Failed` enum — copy this exactly; it's the cleanest fallback contract.
- RTK's `extract_json_object` (handles `dotenv`-style prefixes before JSON) is worth porting verbatim — it's tiny and unblocks every test-runner parser.
- RTK's `package_manager_exec` util (auto-detect pnpm/yarn/npx) is reusable across half a dozen modules — keep it small and dependency-free.
- RTK's `ok_confirmation()` helper (collapses `git add`/`commit`/`push` outputs to "ok abc1234"-style one-liners) is a 30-LOC win that buys 60-80% savings on git ops.
- RTK's `MULTI_BLANK_RE = \n{3,}` collapser for markdown is reusable in `ig pack` and `ig read`.
- RTK's `lazy_static!` REGEX_SET for command classification (`rtk:src/discover/registry.rs`) is faster than ig's current ad-hoc `Vec<Regex>` matching in `rewrite.rs` — switch to `RegexSet` for O(1) set membership.

---

## 8. Out-of-scope / future work

- **Hermes/OpenClaw plugin packaging**: ship after PR 6 as a follow-up. The plugins are TS/Python and don't fit in the Rust binary; needs a separate release workflow.
- **MCP server**: neither RTK nor ig has one yet. A future "agent context server" exposing `ig grep/read/pack/symbols` over MCP would be a 10× differentiator vs RTK, since RTK's filters are inherently CLI-only.
- **LSP integration**: out of scope — neither tool has this.
- **Windows native parity**: same RTK posture; full WSL support, degraded native support.

---

## 9. Quick reference for the implementer

- Always anchor regex on `^`; always strip ANSI first.
- Never `unwrap()` on `Command::output()`; always propagate exit codes.
- Every new module: one test fixture (`tests/fixtures/<tool>/raw.txt` + `expected.compact.txt` + `expected.ultra.txt`).
- TOML inline `[[filters.<id>.tests]]` block is required for every TOML filter touched.
- `cargo test --quiet && cargo clippy --all-targets -- -D warnings && cargo fmt --check` is the merge gate (per CLAUDE.md project policy).
- For each PR: `cp target/release/ig ~/.local/bin/ig && codesign -fs - -i dev.makfly.ig ~/.local/bin/ig` (macOS) before declaring done.

---

End of SPEC-rtk-iso-plan.md.
