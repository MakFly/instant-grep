#!/usr/bin/env bash
# ig Claude Code hook — rewrites file-exploration commands to use ig for token savings.
# Install: ig setup (auto-installs this hook)
#
# Exit code protocol (same as RTK):
#   JSON with updatedInput → rewrite + auto-allow
#   exit 0 (no output)     → passthrough unchanged

if ! command -v jq &>/dev/null; then
  exit 0
fi

if ! command -v ig &>/dev/null; then
  # Try common install locations
  for p in ~/.local/bin/ig /usr/local/bin/ig /opt/homebrew/bin/ig; do
    [ -x "$p" ] && { IG="$p"; break; }
  done
  [ -z "$IG" ] && exit 0
else
  IG=ig
fi

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

[ -z "$CMD" ] && exit 0

# Delegate rewrite logic to the ig binary
REWRITTEN=$($IG rewrite "$CMD" 2>/dev/null)
EXIT_CODE=$?

# Exit 1 = no rewrite available
[ $EXIT_CODE -ne 0 ] && exit 0

# Exit 0 = rewrite found. If identical, skip.
[ "$CMD" = "$REWRITTEN" ] && exit 0

ORIGINAL_INPUT=$(echo "$INPUT" | jq -c '.tool_input')
UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')

jq -n \
  --argjson updated "$UPDATED_INPUT" \
  '{
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": "allow",
      "permissionDecisionReason": "ig auto-rewrite",
      "updatedInput": $updated
    }
  }'
