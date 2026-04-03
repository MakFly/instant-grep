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
    data = json.load(sys.stdin)
    memories = data.get('data', {}).get('memories', [])
    if not memories:
        sys.exit(0)
    lines = ['[brain.dev] Relevant project memory:']
    for m in memories[:3]:
        lines.append(f\"  - {m.get('content', '')[:200]}\")
    print('\n'.join(lines))
except:
    sys.exit(0)
" 2>/dev/null)

[ -z "$CTX" ] && exit 0

# Output additionalContext for Claude Code
python3 -c "import json,sys; print(json.dumps({'additionalContext': sys.argv[1]}))" "$CTX"
