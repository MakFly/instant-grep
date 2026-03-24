#!/bin/bash
# <xbar.title>ig daemon status</xbar.title>
# <xbar.version>v1.0</xbar.version>
# <xbar.author>kev</xbar.author>
# <xbar.desc>Shows instant-grep daemon status for active projects</xbar.desc>

IG=~/.local/bin/ig
ICON_RUNNING="🔍"
ICON_STOPPED="⊘"

# Find all running ig daemon sockets
SOCKETS=$(ls /tmp/ig-*.sock 2>/dev/null)

if [ -z "$SOCKETS" ]; then
    echo "$ICON_STOPPED ig"
    echo "---"
    echo "No ig daemons running | color=gray"
    echo "---"
    echo "Start daemon in a project: ig daemon start | font=Monaco size=11"
    exit 0
fi

# Count running daemons
COUNT=$(echo "$SOCKETS" | wc -l | tr -d ' ')
echo "$ICON_RUNNING $COUNT"
echo "---"

# For each project with an ig index
for PID_FILE in $(find ~ -maxdepth 6 -path '*/.ig/daemon.pid' 2>/dev/null); do
    IG_DIR=$(dirname "$PID_FILE")
    PROJECT_DIR=$(dirname "$IG_DIR")
    PROJECT_NAME=$(basename "$PROJECT_DIR")
    PID=$(cat "$PID_FILE" 2>/dev/null)

    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        # Get file count from metadata
        FILE_COUNT=""
        if [ -f "$IG_DIR/metadata.json" ]; then
            FILE_COUNT=$(python3 -c "import json; m=json.load(open('$IG_DIR/metadata.json')); print(m.get('file_count','?'))" 2>/dev/null)
        fi
        echo "● $PROJECT_NAME — $FILE_COUNT files (PID $PID) | color=green font=Monaco size=12"
        echo "--Stop | bash=$IG shell=/bin/bash param1=daemon param2=stop param3=$PROJECT_DIR terminal=false refresh=true"
        echo "--Open in Terminal | bash=/bin/zsh param1=-c param2='cd $PROJECT_DIR && exec zsh' terminal=true"
    fi
done

echo "---"

# Show projects with index but no daemon
for META in $(find ~ -maxdepth 6 -path '*/.ig/metadata.bin' 2>/dev/null); do
    IG_DIR=$(dirname "$META")
    PROJECT_DIR=$(dirname "$IG_DIR")
    PROJECT_NAME=$(basename "$PROJECT_DIR")
    PID_FILE="$IG_DIR/daemon.pid"

    if [ ! -f "$PID_FILE" ] || ! kill -0 "$(cat "$PID_FILE" 2>/dev/null)" 2>/dev/null; then
        echo "○ $PROJECT_NAME (indexed, daemon off) | color=gray font=Monaco size=12"
        echo "--Start daemon | bash=$IG shell=/bin/bash param1=daemon param2=start param3=$PROJECT_DIR terminal=false refresh=true"
    fi
done

echo "---"
echo "Refresh | refresh=true"
