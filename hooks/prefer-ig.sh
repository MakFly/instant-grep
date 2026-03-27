#!/bin/bash
# Block grep/rg/find for code search — force ig usage.
# Piped grep (echo|grep, cat|grep) and inline string tests are allowed.
# Hooks in settings.json use grep internally — they run outside this scope.

COMMAND="$CLAUDE_BASH_COMMAND"

# ── Allow: piped grep (echo/cat/printf ... | grep) ──
# These are string manipulation, not code search.
if echo "$COMMAND" | grep -qE '\|[[:space:]]*(grep|egrep|fgrep)'; then
  exit 0
fi

# ── Block: rg (always) ──
if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)rg\s'; then
  echo "BLOCK: Use ig instead of rg. Examples:" >&2
  echo "  ig \"pattern\" [path]        # search" >&2
  echo "  ig -l \"pattern\"            # file paths only" >&2
  exit 2
fi

# ── Block: grep used for code search ──
# grep -r, grep -rn, grep -R, grep --include, grep --recursive, egrep, fgrep
if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)(grep\s+-(r|R|rn|nr|rl|lr)|grep\s+--include|grep\s+--recursive|egrep\s|fgrep\s)'; then
  echo "BLOCK: Use ig instead of grep for code search. Examples:" >&2
  echo "  ig \"pattern\" [path]        # search" >&2
  echo "  ig -t rs \"pattern\"         # filter by type" >&2
  echo "  ig -i \"pattern\"            # case-insensitive" >&2
  exit 2
fi

# ── Block: find for file discovery ──
# find . -name, find /path -name, find . -type f, find /path -type f
# Allow: find with -maxdepth 1 (basically ls), find ... -delete (cleanup)
if echo "$COMMAND" | grep -qE '(^|\s|;|&&|\|\|)find\s+\S+\s+.*(-name|-type\s+f|-iname)' && \
   ! echo "$COMMAND" | grep -qE 'find\s+\S+\s+-maxdepth\s+1'; then
  echo "BLOCK: Use ig for file discovery. Examples:" >&2
  echo "  ig files [path]            # list all indexed files" >&2
  echo "  ig files | grep '\\.php$'  # filter by extension" >&2
  echo "  ig symbols [path]          # function/class defs" >&2
  echo "  ig -l \"pattern\" [path]    # files matching content" >&2
  exit 2
fi

exit 0
