#!/usr/bin/env bash
# ig-guard.sh — PreToolUse/Bash hook
# RTK-style thin delegator: all rewrite logic lives in `ig rewrite` (the Rust
# binary, src/rewrite.rs). No BLOCK messages — commands are silently
# substituted via updatedInput so the agent never sees friction.
#
# Command source resolution (compatible with both legacy and current Claude
# Code harnesses):
#   1. $CLAUDE_BASH_COMMAND (legacy env var, still honored when available)
#   2. stdin JSON .tool_input.command (Claude Code 2.1+ harness)

set -u

# ── Read stdin once so Phase 1 + Phase 2 can share it ──────────────────────
INPUT=""
if [[ ! -t 0 ]]; then
  INPUT=$(cat)
fi

# ── Resolve the command ─────────────────────────────────────────────────────
COMMAND="${CLAUDE_BASH_COMMAND:-}"
if [[ -z "$COMMAND" && -n "$INPUT" ]] && command -v jq &>/dev/null; then
  COMMAND=$(printf '%s' "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)
fi
[[ -z "$COMMAND" ]] && exit 0

# Skip for subagents — they can use whatever they like.
[[ -n "${CLAUDE_AGENT_NAME:-}" ]] && exit 0

# Rewriting needs stdin JSON (to emit updatedInput) + jq
[[ -z "$INPUT" ]] && exit 0
command -v jq &>/dev/null || exit 0

ORIGINAL_INPUT=$(printf '%s' "$INPUT" | jq -c '.tool_input' 2>/dev/null)
[[ -z "$ORIGINAL_INPUT" ]] && exit 0

# ── Rewrite 1: find → command find (bypass fd alias) ────────────────────────
# Only for find invocations that aren't fully handled by `ig rewrite`
# (i.e. find without -name/-type f).
if echo "$COMMAND" | command grep -qE '(^|[;&|]+[[:space:]]*)find ' && \
   ! echo "$COMMAND" | command grep -qE '\-(name|iname|type[[:space:]]+f)\b'; then
  REWRITTEN=$(echo "$COMMAND" | command sed -E 's/(^|[;&|]+[[:space:]]*)find /\1command find /g')
  if [[ "$COMMAND" != "$REWRITTEN" ]]; then
    UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')
    jq -n --argjson updated "$UPDATED_INPUT" '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "find → command find (bypass fd alias)",
        "updatedInput": $updated
      }
    }'
    exit 0
  fi
fi

# ── Rewrite 2: delegate to `ig rewrite` (Rust binary, src/rewrite.rs) ───────
if command -v ig &>/dev/null; then
  IG=ig
else
  for p in ~/.local/bin/ig /usr/local/bin/ig /opt/homebrew/bin/ig; do
    [[ -x "$p" ]] && { IG="$p"; break; }
  done
  [[ -z "${IG:-}" ]] && exit 0
fi

REWRITTEN=$($IG rewrite "$COMMAND" 2>/dev/null)
EXIT_CODE=$?

# RTK 0/1/2/3 exit-code protocol (see src/rewrite.rs):
#   0 → passthrough (familiar command, nothing to do)
#   1 → rewrite suggestion / ask — surface to user, allow execution
#   2 → deny — block
#   3 → Default + unfamiliar — passthrough; Claude's own permission layer
#       remains the source of truth for execution prompts.
case $EXIT_CODE in
  0)
    # Passthrough — nothing to do.
    exit 0
    ;;
  1)
    # Rewrite suggestion. If unchanged, treat as passthrough.
    if [[ -z "$REWRITTEN" || "$COMMAND" = "$REWRITTEN" ]]; then
      exit 0
    fi
    UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')
    jq -n --argjson updated "$UPDATED_INPUT" '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "ig auto-rewrite",
        "updatedInput": $updated
      }
    }'
    exit 0
    ;;
  2)
    echo "BLOCK (ig rewrite): destructive command refused" >&2
    exit 2
    ;;
  3)
    # Default verdict + unfamiliar command. This hook is only responsible for
    # search-command rewrites and hard deny rules; unknown safe-looking shell
    # scripts should not produce noisy PreToolUse ASK messages.
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
