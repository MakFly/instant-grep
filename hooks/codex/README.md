# Codex CLI

Codex has no native hook system (as of 2026-05). `ig setup --agent codex`
only refreshes the managed-block in `~/.codex/AGENTS.md`.

To get the equivalent of Claude's `ig-guard.sh` rewriting, wrap your `codex`
invocations:

```bash
codex_with_ig() {
  export PATH="$HOME/.local/bin:$PATH"   # ensure `ig` is reachable
  codex "$@"
}
```
