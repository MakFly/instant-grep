#!/usr/bin/env bash
# brain-capture.sh — PostToolUse hook
# Captures Claude Code memory writes and syncs to brain.dev
# Only fires for files in */memory/*.md — ignores regular code edits

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

# ONLY capture memory files — ignore all other edits
case "$FILE" in
  */memory/*.md) ;;
  *) exit 0 ;;
esac

# File must exist to read its content
[ ! -f "$FILE" ] && exit 0

# Extract project name from the path
# e.g. /Users/kev/.claude/projects/-Users-kev-Documents-lab-sandbox-headless-kit/memory/project_status.md
# → project = headless-kit (last segment before /memory/)
PROJECT=$(python3 -c "
import sys, os
path = sys.argv[1]
parts = path.split('/memory/')
if len(parts) >= 2:
    # Get the project dir name, take last meaningful segment
    proj_path = parts[0]
    name = os.path.basename(proj_path)
    # If it's a Claude projects path like -Users-kev-...-project-name, extract last part
    if name.startswith('-'):
        segments = name.split('-')
        # Find meaningful project name (last 1-3 segments, skip Users/kev/Documents/lab/sandbox)
        meaningful = [s for s in segments if s and s.lower() not in ('users', 'kev', 'documents', 'lab', 'sandbox', 'perso')]
        name = '-'.join(meaningful[-3:]) if meaningful else name
    print(name)
else:
    print(os.path.basename(os.getcwd()))
" "$FILE" 2>/dev/null)

# Parse frontmatter and content, then POST to brain.dev
(python3 -c "
import sys, json, re, os
try:
    filepath = sys.argv[1]
    token = sys.argv[2]
    api_url = sys.argv[3]
    project = sys.argv[4]

    with open(filepath, 'r') as f:
        raw = f.read()

    # Parse YAML frontmatter
    name = os.path.splitext(os.path.basename(filepath))[0]
    mem_type = 'project'
    description = ''
    content = raw

    fm_match = re.match(r'^---\s*\n(.*?)\n---\s*\n', raw, re.DOTALL)
    if fm_match:
        fm = fm_match.group(1)
        content = raw[fm_match.end():]
        for line in fm.split('\n'):
            if line.startswith('name:'):
                name = line.split(':', 1)[1].strip()
            elif line.startswith('type:'):
                mem_type = line.split(':', 1)[1].strip()
            elif line.startswith('description:'):
                description = line.split(':', 1)[1].strip()

    import urllib.request
    payload = json.dumps({
        'name': name,
        'type': mem_type,
        'description': description,
        'content': content.strip(),
        'project': project,
    }).encode()

    req = urllib.request.Request(
        f'{api_url}/brain/memories/sync',
        data=payload,
        headers={
            'Authorization': f'Bearer {token}',
            'Content-Type': 'application/json',
        },
        method='POST',
    )
    urllib.request.urlopen(req, timeout=5)
except:
    pass
" "$FILE" "$TOKEN" "$API_URL" "$PROJECT" &) 2>/dev/null

exit 0
