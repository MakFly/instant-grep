#!/usr/bin/env bash
# brain-inject.sh — UserPromptSubmit hook
# Fetches relevant memories from brain.dev and injects as additionalContext

CONFIG="$HOME/.config/brain/config.json"
[ ! -f "$CONFIG" ] && exit 0

TOKEN=$(python3 -c "import sys,json; print(json.load(open('$CONFIG')).get('token',''))" 2>/dev/null)
API_URL=$(python3 -c "import sys,json; print(json.load(open('$CONFIG')).get('api_url',''))" 2>/dev/null)
[ -z "$TOKEN" ] && exit 0

# Read prompt from stdin
INPUT=$(cat)
PROMPT=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('prompt',''))" 2>/dev/null)
[ -z "$PROMPT" ] && exit 0

# Search memories (2s timeout)
ESCAPED_PROMPT=$(python3 -c "import json,sys; print(json.dumps(sys.argv[1]))" "$PROMPT" 2>/dev/null)
RESULT=$(curl -sS --max-time 2 \
  "$API_URL/brain/search" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"query\":$ESCAPED_PROMPT}" 2>/dev/null)

[ -z "$RESULT" ] && exit 0

# Extract context text
CTX=$(echo "$RESULT" | python3 -c "
import sys, json
try:
    resp = json.load(sys.stdin)
    items = resp.get('data', [])
    if isinstance(items, dict):
        items = items.get('memories', [])
    if not items:
        sys.exit(0)
    lines = ['[brain.dev] Relevant project memory:']
    for item in items[:3]:
        mem = item.get('memory', item)
        lines.append(f\"  - {mem.get('content', '')[:200]}\")
    print('\n'.join(lines))
except:
    sys.exit(0)
" 2>/dev/null)

[ -z "$CTX" ] && exit 0

# Output additionalContext for Claude Code
python3 -c "import json,sys; print(json.dumps({'additionalContext': sys.argv[1]}))" "$CTX"

# Track injection in brain history (background, non-blocking)
MEMORY_COUNT=$(echo "$RESULT" | python3 -c "
import sys, json
try:
    resp = json.load(sys.stdin)
    items = resp.get('data', [])
    if isinstance(items, dict):
        items = items.get('memories', [])
    print(len(items))
except:
    print(0)
" 2>/dev/null)
HISTORY_FILE="$HOME/Library/Application Support/ig/brain-history.jsonl"
(mkdir -p "$(dirname "$HISTORY_FILE")" && \
  echo "{\"ts\":$(date +%s),\"type\":\"inject\",\"memories\":${MEMORY_COUNT:-0},\"est_saved\":5000}" >> "$HISTORY_FILE" &) 2>/dev/null

exit 0
