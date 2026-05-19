# Claude Code hook scripts

Installed by `ig setup --agent claude` into `~/.claude/hooks/`.

| Script | Event | Purpose |
| --- | --- | --- |
| `ig-guard.sh` | `PreToolUse/Bash` | Block / rewrite `grep`/`rg`/`find`/`cat` to `ig`. |
| `format.sh` | `PostToolUse/Write\|Edit` | Project-formatter dispatch (cargo fmt, prettier, ruff, …). |
| `subagent-context.sh` | `SubagentStart` | Inject a 1-line "use ig" reminder into spawned subagents. |

## Disabling

Remove the corresponding entry from `~/.claude/settings.json` (the array under
`hooks.<Event>`), or run `ig setup --uninstall --agent claude`.

## Auditing

`ig hook-audit` shows the last 7 days of hook decisions (blocked / allowed /
rewritten) for the destructive-git, npm/npx and grep guards.
