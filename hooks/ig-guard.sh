#!/usr/bin/env bash
# ig-guard.sh — PreToolUse/Bash hook
# Phase 1: no-op early exits (subagent pass-through, empty command).
# Phase 2: full rewrite / deny logic delegated to `ig rewrite`.
#
# Previously Phase 1 BLOCKED grep/rg/find so Claude saw an error and had to
# retype. That friction is removed: grep/rg/find now fall through to Phase 2
# where `ig rewrite` transparently substitutes the command via updatedInput.
# The user never sees a block, the agent keeps going.

COMMAND="$CLAUDE_BASH_COMMAND"
[[ -z "$COMMAND" ]] && exit 0

# Skip for subagents — they can use whatever they like.
[[ -n "${CLAUDE_AGENT_NAME:-}" ]] && exit 0

# ── Phase 2: Rewriting + DENY enforcement ──────────────────────────────────
# Uses stdin JSON — needs jq.

command -v jq &>/dev/null || exit 0

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')
[[ -z "$CMD" ]] && exit 0

ORIGINAL_INPUT=$(echo "$INPUT" | jq -c '.tool_input')

# 2a. find → command find (bypass fd alias)
if echo "$CMD" | command grep -qE '(^|[;&|]+[[:space:]]*)find '; then
  REWRITTEN=$(echo "$CMD" | command sed -E 's/(^|[;&|]+[[:space:]]*)find /\1command find /g')
  if [[ "$CMD" != "$REWRITTEN" ]]; then
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

# 2b. ig rewrite (delegated to ig binary)
if command -v ig &>/dev/null; then
  IG=ig
else
  for p in ~/.local/bin/ig /usr/local/bin/ig /opt/homebrew/bin/ig; do
    [[ -x "$p" ]] && { IG="$p"; break; }
  done
  [[ -z "${IG:-}" ]] && exit 0
fi

REWRITTEN=$($IG rewrite "$CMD" 2>/dev/null)
EXIT_CODE=$?

case $EXIT_CODE in
  0) # Rewrite found — auto-allow
    [[ "$CMD" = "$REWRITTEN" ]] && exit 0
    UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')
    jq -n --argjson updated "$UPDATED_INPUT" '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "ig auto-rewrite",
        "updatedInput": $updated
      }
    }'
    ;;
  2) # Deny — destructive command. The Rust rewriter wrote the reason to
     # its stderr, which was swallowed; re-emit a clear message to the
     # user and block the tool invocation.
    echo "BLOCK (ig rewrite): destructive command refused" >&2
    exit 2
    ;;
  3) # Rewrite needs user confirmation
    UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')
    jq -n --argjson updated "$UPDATED_INPUT" '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": $updated
      }
    }'
    ;;
  *) exit 0 ;;
esac
