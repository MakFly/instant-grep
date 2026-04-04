#!/bin/bash
# SessionStart hook — pull brain.dev context to local files
# Runs at the start of each Claude Code session
# Timeout: 5s (must never block Claude startup)

CONFIG="$HOME/.config/brain/config.json"
[ ! -f "$CONFIG" ] && exit 0

# Skip if last pull < 5 minutes ago
CACHE="$HOME/.config/brain/.last_pull"
if [ -f "$CACHE" ]; then
  LAST=$(cat "$CACHE")
  NOW=$(date +%s)
  [ $((NOW - LAST)) -lt 300 ] && exit 0
fi

# Pull brain.dev context (skills, rules, memories summary)
ig brain pull --quiet 2>/dev/null

# Update cache timestamp
mkdir -p "$(dirname "$CACHE")"
date +%s > "$CACHE"

exit 0
