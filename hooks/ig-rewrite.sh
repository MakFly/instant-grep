#!/usr/bin/env bash
# ig Claude Code hook — rewrites file-exploration commands to use ig for token savings.
# Install: ig setup (auto-installs this hook)
#
# Exit code protocol (same as RTK):
#   0 + stdout  → rewrite found, auto-allow
#   1           → no rewrite, passthrough
#   2           → deny, reason on stderr (let Claude Code's native deny handle it)
#   3 + stdout  → rewrite found, require user confirmation

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
REWRITE_ERR=$(mktemp)
REWRITTEN=$($IG rewrite "$CMD" 2>"$REWRITE_ERR")
EXIT_CODE=$?

ORIGINAL_INPUT=$(echo "$INPUT" | jq -c '.tool_input')
UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')

case $EXIT_CODE in
  0)
    # Rewrite found — auto-allow
    rm -f "$REWRITE_ERR"
    [ "$CMD" = "$REWRITTEN" ] && exit 0
    ;;
  1)
    # No rewrite — passthrough
    rm -f "$REWRITE_ERR"
    exit 0
    ;;
  2)
    # Deny — block destructive command, forward reason to stderr
    cat "$REWRITE_ERR" >&2
    rm -f "$REWRITE_ERR"
    exit 2
    ;;
  3)
    # Ask — rewrite but don't auto-allow (user confirms)
    rm -f "$REWRITE_ERR"
    ;;
  *)
    rm -f "$REWRITE_ERR"
    exit 0
    ;;
esac

if [ "$EXIT_CODE" -eq 3 ]; then
  jq -n \
    --argjson updated "$UPDATED_INPUT" \
    '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": $updated
      }
    }'
else
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
fi
