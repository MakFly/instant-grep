#!/usr/bin/env bash
# Rewrites `find` to `command find` to bypass the fd alias.
# Exit codes: 0 + stdout → rewrite + auto-allow, 1 → passthrough

command -v jq &>/dev/null || exit 1

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

[ -z "$CMD" ] && exit 1

# Check if the command starts with `find ` or contains ` find ` (in a chain)
# but NOT `command find` already
if echo "$CMD" | command grep -qE '(^|[;&|]+[[:space:]]*)find '; then
  # Replace `find ` with `command find ` (standalone, including after && || ; |)
  REWRITTEN=$(echo "$CMD" | command sed -E 's/(^|[;&|]+[[:space:]]*)find /\1command find /g')

  [ "$CMD" = "$REWRITTEN" ] && exit 1

  ORIGINAL_INPUT=$(echo "$INPUT" | jq -c '.tool_input')
  UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')

  jq -n \
    --argjson updated "$UPDATED_INPUT" \
    '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "find → command find (bypass fd alias)",
        "updatedInput": $updated
      }
    }'
else
  exit 1
fi
