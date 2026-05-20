# Migrating from RTK to `ig`

`ig` (instant-grep) v2.0.0 ships feature parity with [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk):
the same command-output filtering that cuts LLM token spend on `ls`, `git`,
test runners, linters, cloud CLIs, and more — plus a trigram-indexed code
search engine RTK does not have.

This guide covers moving an existing RTK setup to `ig`.

## Why migrate

- **Everything RTK does**, plus a real search engine: `ig "regex" [path]` is
  trigram-indexed, sub-millisecond on a warm index, with `rg`-parity matching.
- `ig read -s` (signatures only), `ig smart`, `ig symbols`, `ig pack`, and
  `ig context <file> <line>` reuse that index to compress file reads — RTK has
  no equivalent.
- One static Rust binary, no daemon, no background process.
- Opt-in telemetry that ships **no endpoint URL** in the open-source build.

## Commands map (rtk → ig)

The CLI surface is intentionally close. In most cases, replace `rtk` with `ig`.

| RTK                       | `ig`                          |
| ------------------------- | ----------------------------- |
| `rtk ls [path]`           | `ig ls [path]`                |
| `rtk read <file>`         | `ig read <file>` (`-s` for signatures) |
| `rtk grep <pat>`          | `ig "<pat>" [path]`           |
| `rtk find <pat>`          | `ig files` / `ig "<pat>"`     |
| `rtk git <args>`          | `ig git <args>`               |
| `rtk diff a b`            | `ig diff a b`                 |
| `rtk gh pr list`          | `ig gh pr list`               |
| `rtk vitest` / `jest` …   | `ig vitest` / `ig jest` …     |
| `rtk pytest` / `go test`  | `ig pytest` / `ig go-test`    |
| `rtk eslint` / `tsc` …    | `ig eslint` / `ig tsc` …      |
| `rtk aws <svc> <verb>`    | `ig aws <svc> <verb>`         |
| `rtk kubectl get pods`    | `ig kubectl get pods`         |
| `rtk run <cmd>`           | `ig run <cmd>` (alias `ig proxy`) |
| `rtk gain`                | `ig gain`                     |
| `rtk discover`            | `ig discover`                 |
| `rtk init`                | `ig setup` (alias `ig init`)  |

Global flags: `-u/--ultra-compact` and `-v/-vv/-vvv` work the same. `ig` adds
`--explain` (show the filter / rewrite decision) and `--semantic` (PMI synonym
expansion on search).

## Filter import

If you have custom RTK filters, import them in one shot:

```bash
ig import-rtk            # translate ~/.config/rtk/filters.toml + ./.rtk/filters.toml
ig import-rtk --dry-run  # preview without writing
```

This writes `~/.config/ig/filters/imported-from-rtk.toml` (user-level) and
`<project>/.ig/filters/imported-from-rtk.toml` (project-level, when the project
is a git repo). Only the **project-local** file is auto-trusted — review and
`ig trust` the user-level file yourself if you want it active.

RTK filter fields map 1:1 (`match`, `strip_ansi`, `keep`, `drop`, `truncate`,
`head`, `tail`, `max_lines`, `on_empty`, `dedup_consecutive`). RTK features
that have no TOML equivalent (state machines, NDJSON streaming) are skipped
with a `# (rtk-only feature: …)` comment — `ig` already covers those tools
with native Rust parsers, so the filter is not needed.

You can also run it as part of agent setup: `ig setup --import-rtk`.

## Hook re-installation

RTK's PreToolUse hooks must be replaced with `ig`'s. Run:

```bash
ig setup                 # all detected agents
ig setup --agent claude  # one agent
ig setup --show          # list what is installed
```

`ig` supports 11 agents: claude, codex, cursor, copilot, gemini, opencode,
windsurf, cline, hermes, kilocode, antigravity.

The hook contract uses the RTK 0/1/2/3 exit-code protocol, so behaviour is
identical. After installing `ig`'s hooks you can remove RTK's — `ig uninstall`
only touches `ig` artifacts, so it will not clean RTK's; remove those with
`rtk` itself before switching.

## Tracking DB migration

`ig` keeps its own SQLite tracking DB (`<data-dir>/ig/tracking.db`). It does
**not** import RTK's `history.db` — savings history starts fresh. `ig gain`,
`ig discover`, and `ig economics` populate from the first `ig`-routed command
onward. If you previously used `ig` v1.x, its JSONL history is migrated to
SQLite automatically on first run.

## RTK was never daemon-based

RTK runs process-per-invocation, and so does `ig` v2.0.0 — there is no daemon
migration step. (Earlier `ig` v1.x had a daemon; `ig update` cleans up its
leftovers automatically. This does not affect RTK users.)

## FAQ

**Can I run `ig` and `rtk` side by side?**
Yes, but only one set of agent hooks can be active at a time — the last
`init`/`setup` wins. Pick one.

**Does `ig` send telemetry like RTK?**
Only if you explicitly run `ig telemetry consent --yes`, and only if the
binary was built with an endpoint URL (the public release is not). Default is
fully offline. `IG_TELEMETRY_DISABLED=1` is a hard kill switch.

**My RTK filter used a streaming strategy — where did it go?**
`ig` has a native Rust parser for that tool (`ig vitest`, `ig go-test`, etc.).
The streaming TOML filter is unnecessary; just call the `ig` subcommand.

**How do I see what `ig` would do without changing behaviour?**
`ig --explain run <cmd>` prints the filter / rewrite decision.
