#!/bin/bash
# Claude Code PostToolUse hook — auto-format edited files
# Uses local tooling only (no bunx/npx downloads). Exits fast for unsupported extensions.

FILE="$CLAUDE_FILE_PATH"
[[ -z "$FILE" || ! -f "$FILE" ]] && exit 0

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || echo "$CLAUDE_WORKING_DIR")"

case "$FILE" in
  *.js|*.jsx|*.ts|*.tsx|*.json|*.css|*.scss|*.md|*.yaml|*.yml)
    if [[ -f "$ROOT/node_modules/.bin/prettier" ]]; then
      "$ROOT/node_modules/.bin/prettier" --write "$FILE" 2>/dev/null || true
    else
      MARKER="/tmp/.claude-no-prettier-$(echo "$ROOT" | md5 2>/dev/null || md5sum <<< "$ROOT" | cut -d' ' -f1)"
      if [[ ! -f "$MARKER" ]]; then
        echo "INFO: prettier not found in $ROOT — run 'bun add -d prettier' to enable auto-formatting." >&2
        touch "$MARKER"
      fi
    fi
    ;;
  *.php)
    if [[ -f "$ROOT/vendor/bin/pint" ]]; then
      "$ROOT/vendor/bin/pint" "$FILE" 2>/dev/null || true
    elif [[ -f "$ROOT/apps/api/vendor/bin/pint" ]]; then
      "$ROOT/apps/api/vendor/bin/pint" "$FILE" 2>/dev/null || true
    elif [[ -f "$ROOT/vendor/bin/php-cs-fixer" ]]; then
      "$ROOT/vendor/bin/php-cs-fixer" fix "$FILE" --diff 2>/dev/null || true
    elif command -v php-cs-fixer &>/dev/null; then
      php-cs-fixer fix "$FILE" --quiet 2>/dev/null || true
    fi
    ;;
  *.go)
    command -v gofmt &>/dev/null && gofmt -w "$FILE" 2>/dev/null || true
    ;;
  *.rs)
    command -v rustfmt &>/dev/null && rustfmt "$FILE" 2>/dev/null || true
    ;;
  *.py)
    command -v ruff &>/dev/null && ruff format "$FILE" 2>/dev/null || true
    ;;
esac

exit 0
