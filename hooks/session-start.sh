#!/opt/homebrew/bin/bash
set -u

CACHE_DIR="${HOME}/.claude/cache"
mkdir -p "$CACHE_DIR"

# ── 1. Version check ──
LAST_VERSION_FILE="${CACHE_DIR}/.last-version"
CURRENT_VERSION=$(claude --version 2>/dev/null | head -1 | sed 's/ .*//')

if [[ -n "$CURRENT_VERSION" ]]; then
  LAST_VERSION=""
  [[ -f "$LAST_VERSION_FILE" ]] && LAST_VERSION=$(cat "$LAST_VERSION_FILE")

  if [[ "$CURRENT_VERSION" != "$LAST_VERSION" && -n "$LAST_VERSION" ]]; then
    echo "" >&2
    echo "━━━ Claude Code updated: ${LAST_VERSION} → ${CURRENT_VERSION} ━━━" >&2

    CHANGELOG="${CACHE_DIR}/changelog.md"
    if [[ -f "$CHANGELOG" ]]; then
      awk -v ver="## ${CURRENT_VERSION}" '
        $0 ~ ver { found=1; next }
        found && /^## / { exit }
        found && /^- / { count++; if (count <= 5) print "  " $0 }
        END { if (count > 5) print "  ... and " (count-5) " more changes" }
      ' "$CHANGELOG" >&2
    fi

    echo "" >&2
    echo "  Run /release-notes for full changelog" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
  fi

  echo "$CURRENT_VERSION" > "$LAST_VERSION_FILE"
fi

# ── 2. ig session lock + gain one-liner ──
if command -v ig &>/dev/null; then
  PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$PWD}"
  ig hold begin "$PROJECT_DIR" >/dev/null 2>&1 || true
  if command -v jq &>/dev/null; then
    GAIN_JSON=$(ig gain --json 2>/dev/null | head -1)
    if [[ -n "$GAIN_JSON" ]]; then
      SAVED=$(echo "$GAIN_JSON" | jq -r '.total_tokens_saved // empty' 2>/dev/null)
      if [[ -n "$SAVED" && "$SAVED" != "0" && "$SAVED" != "null" ]]; then
        echo "ig: ${SAVED} tokens saved (ig gain for details)" >&2
      fi
    fi
  fi
fi

# ── 3. Cleanup reminder ──
LAST_CLEANUP_FILE="${CACHE_DIR}/.last-cleanup"

if [[ -f "$LAST_CLEANUP_FILE" ]]; then
  LAST_CLEANUP=$(cat "$LAST_CLEANUP_FILE")
  NOW=$(date +%s)
  ELAPSED=$(( (NOW - LAST_CLEANUP) / 86400 ))
  if [[ "$ELAPSED" -ge 7 ]]; then
    DEBRIS_KB=0
    for d in telemetry debug paste-cache; do
      if [[ -d "${HOME}/.claude/${d}" ]]; then
        DEBRIS_KB=$(( DEBRIS_KB + $(du -sk "${HOME}/.claude/${d}" 2>/dev/null | cut -f1) ))
      fi
    done
    if [[ "$DEBRIS_KB" -gt 10240 ]]; then
      DEBRIS_MB=$(( DEBRIS_KB / 1024 ))
      echo "cleanup: ${DEBRIS_MB}MB reclaimable — run claude-cleanup --force (${ELAPSED}d since last)" >&2
    fi
  fi
else
  date +%s > "$LAST_CLEANUP_FILE"
fi

exit 0
