#!/usr/bin/env bash
# brain-capture.sh — PostToolUse hook
# Auto-captures observations after successful edits (fire-and-forget)

CONFIG="$HOME/.config/brain/config.json"
[ ! -f "$CONFIG" ] && exit 0

TOKEN=$(python3 -c "import sys,json; print(json.load(open('$CONFIG')).get('token',''))" 2>/dev/null)
API_URL=$(python3 -c "import sys,json; print(json.load(open('$CONFIG')).get('api_url',''))" 2>/dev/null)
[ -z "$TOKEN" ] && exit 0

INPUT=$(cat)
TOOL=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_name',''))" 2>/dev/null)

# Only capture Edit and Write operations
[ "$TOOL" != "Edit" ] && [ "$TOOL" != "Write" ] && exit 0

FILE=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_input',{}).get('file_path',''))" 2>/dev/null)
[ -z "$FILE" ] && exit 0

PROJECT=$(basename "$(pwd)")

# Fire-and-forget background POST
(curl -sS --max-time 5 \
  "$API_URL/brain/capture" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"content\":\"Modified $FILE\",\"project\":\"$PROJECT\",\"source\":\"auto-capture\"}" &) 2>/dev/null

exit 0
