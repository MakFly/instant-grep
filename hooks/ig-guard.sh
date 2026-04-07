#!/usr/bin/env bash
# ig-guard.sh — PreToolUse/Bash hook
# Merged from: ig-rewrite.sh, find-rewrite.sh, prefer-ig.sh
# Phase 1: block forbidden search tools (from env var, fast, no deps)
# Phase 2: rewrite commands to ig equivalents (from stdin JSON, needs jq)

# ── Phase 1: Blocking ───────────────────────────────────────────────────────
# Uses $CLAUDE_BASH_COMMAND (env var) — no stdin read, instant exit on block.

COMMAND="$CLAUDE_BASH_COMMAND"
[[ -z "$COMMAND" ]] && exit 0

# Skip blocking for subagents — they need find/grep for exploration
[[ -n "${CLAUDE_AGENT_NAME:-}" ]] && exit 0

# Allow: piped grep (echo|grep, cat|grep) — string manipulation, not code search
if ! echo "$COMMAND" | grep -qE '\|[[:space:]]*(grep|egrep|fgrep)'; then

  # Block: rg (always)
  if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)rg\s'; then
    echo "BLOCK: Use ig instead of rg. Examples:" >&2
    echo "  ig \"pattern\" [path]        # search" >&2
    echo "  ig -l \"pattern\"            # file paths only" >&2
    exit 2
  fi

  # Block: grep used for code search (grep -r, -R, -rn, --include, --recursive, egrep, fgrep)
  if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)(grep\s+-(r|R|rn|nr|rl|lr)|grep\s+--include|grep\s+--recursive|egrep\s|fgrep\s)'; then
    echo "BLOCK: Use ig instead of grep for code search. Examples:" >&2
    echo "  ig \"pattern\" [path]        # search" >&2
    echo "  ig -t rs \"pattern\"         # filter by type" >&2
    echo "  ig -i \"pattern\"            # case-insensitive" >&2
    exit 2
  fi

  # Block: find for file discovery (allow -maxdepth 1 which is basically ls)
  if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)find\s+\S+\s+.*(-name|-type\s+f|-iname)' && \
     ! echo "$COMMAND" | grep -qE 'find\s+\S+\s+-maxdepth\s+1'; then
    echo "BLOCK: Use ig for file discovery. Examples:" >&2
    echo "  ig files [path]            # list all indexed files" >&2
    echo "  ig -l \"pattern\" [path]    # files matching content" >&2
    exit 2
  fi

fi

# ── Phase 2: Rewriting ──────────────────────────────────────────────────────
# Uses stdin JSON — only reached if Phase 1 didn't block.

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
