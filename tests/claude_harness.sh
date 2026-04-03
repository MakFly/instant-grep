#!/usr/bin/env bash
# claude_harness.sh — Claude Code CLI integration tests for ig
# Inspired by multica (https://github.com/multica-ai/multica)
#
# Each test creates a temp fixture, indexes it, then asks Claude Code
# to use ig and validates the JSON response.
#
# Usage:
#   ./tests/claude_harness.sh              # run all tests
#   ./tests/claude_harness.sh C05          # filter by test ID
#   ./tests/claude_harness.sh "regex"      # filter by keyword
#   IG=/path/to/ig ./tests/claude_harness.sh  # custom ig binary

set -o pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
PASS=0; FAIL=0; SKIP=0; ERRORS=()
IG="${IG:-ig}"
FILTER="${1:-}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

MASTER_TMP=$(mktemp -d)
trap "rm -rf $MASTER_TMP" EXIT

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
check_prereqs() {
    local missing=0
    for cmd in claude jq "$IG"; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            echo "ERROR: $cmd not found in PATH"
            missing=1
        fi
    done
    [[ $missing -eq 1 ]] && exit 1
}

# ---------------------------------------------------------------------------
# Timeout (macOS-compatible, no coreutils needed)
# ---------------------------------------------------------------------------
with_timeout() {
    local secs=$1; shift
    perl -e 'alarm shift @ARGV; exec @ARGV' -- "$secs" "$@"
}

# ---------------------------------------------------------------------------
# Create a fresh git-initialized fixture project
# ---------------------------------------------------------------------------
new_project() {
    local dir="$MASTER_TMP/proj_$$_$RANDOM"
    mkdir -p "$dir"
    (cd "$dir" && git init -q && git commit --allow-empty -m "init" -q)
    echo "$dir"
}

# ---------------------------------------------------------------------------
# Ask Claude Code CLI and return raw JSON
# ---------------------------------------------------------------------------
ask_claude() {
    local prompt="$1"
    with_timeout 120 claude -p "$prompt" \
        --output-format json \
        --permission-mode bypassPermissions \
        --allowedTools "Bash(ig *),Bash(ls *),Read" \
        --max-turns 5 2>/dev/null
}

# ---------------------------------------------------------------------------
# Test runner
# ---------------------------------------------------------------------------
run_claude_test() {
    local id="$1" desc="$2" prompt="$3" assert_fn="$4"

    # Filter support
    if [[ -n "$FILTER" ]] && ! echo "$id $desc" | grep -qi "$FILTER"; then
        return
    fi

    printf "  %-6s %s ... " "$id" "$desc"

    local raw_output
    raw_output=$(ask_claude "$prompt")
    local exit_code=$?

    local result
    result=$(echo "$raw_output" | jq -r '.result // empty' 2>/dev/null)

    if [[ $exit_code -ne 0 ]]; then
        echo -e "${RED}FAIL${NC} (claude exit $exit_code)"
        ((FAIL++))
        ERRORS+=("$id: $desc (claude error, exit $exit_code)")
        return
    fi

    if [[ -z "$result" ]]; then
        echo -e "${RED}FAIL${NC} (empty result)"
        ((FAIL++))
        ERRORS+=("$id: $desc (empty result from claude)")
        return
    fi

    if eval "$assert_fn"; then
        echo -e "${GREEN}PASS${NC}"
        ((PASS++))
    else
        echo -e "${RED}FAIL${NC}"
        ((FAIL++))
        ERRORS+=("$id: $desc (assertion failed)")
    fi
}

# ---------------------------------------------------------------------------
# Assertion helpers
# ---------------------------------------------------------------------------
assert_contains() {
    echo "$result" | grep -qi "$1"
}

assert_not_contains() {
    ! echo "$result" | grep -qi "$1"
}

assert_contains_all() {
    for pat in "$@"; do
        echo "$result" | grep -qi "$pat" || return 1
    done
}

# ===========================================================================
# TESTS
# ===========================================================================

echo -e "\n${BOLD}=== Claude Code + ig Integration Tests ===${NC}\n"
check_prereqs

# ---------------------------------------------------------------------------
# C01 — Basic search
# ---------------------------------------------------------------------------
test_C01() {
    local dir; dir=$(new_project)
    echo "hello world" > "$dir/foo.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig 'hello' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains_all "hello" "foo.txt"'
    run_claude_test "C01" "Basic search" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C02 — Regex search
# ---------------------------------------------------------------------------
test_C02() {
    local dir; dir=$(new_project)
    printf 'fn main() {\n}\nfn helper() {\n}\n' > "$dir/code.rs"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig '(?m)^fn ' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains_all "main" "helper"'
    run_claude_test "C02" "Regex search (multiline anchor)" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C03 — Case insensitive
# ---------------------------------------------------------------------------
test_C03() {
    local dir; dir=$(new_project)
    echo "ERROR occurred in module" > "$dir/log.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -i 'error' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "error"'
    run_claude_test "C03" "Case insensitive search" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C04 — Fixed strings
# ---------------------------------------------------------------------------
test_C04() {
    local dir; dir=$(new_project)
    echo "version 1.2.3" > "$dir/ver.txt"
    echo "version 1X2Y3" > "$dir/other.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -F '1.2.3' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "ver.txt" && assert_not_contains "other.txt"'
    run_claude_test "C04" "Fixed string search" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C05 — Word boundary
# ---------------------------------------------------------------------------
test_C05() {
    local dir; dir=$(new_project)
    echo "foo bar" > "$dir/yes.txt"
    echo "foobar" > "$dir/no.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -w 'foo' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "yes.txt" && assert_not_contains "no.txt"'
    run_claude_test "C05" "Word boundary search" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C06 — Count mode
# ---------------------------------------------------------------------------
test_C06() {
    local dir; dir=$(new_project)
    echo "import os" > "$dir/a.py"
    echo "import sys" > "$dir/b.py"
    echo "import json" > "$dir/c.py"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -c 'import' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='echo "$result" | grep -qE "[0-9]"'
    run_claude_test "C06" "Count mode" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C07 — Files only
# ---------------------------------------------------------------------------
test_C07() {
    local dir; dir=$(new_project)
    echo "async function fetch() {}" > "$dir/api.js"
    echo "async function save() {}" > "$dir/db.js"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -l 'async' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains_all "api.js" "db.js"'
    run_claude_test "C07" "Files-only mode" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C08 — Type filter
# ---------------------------------------------------------------------------
test_C08() {
    local dir; dir=$(new_project)
    echo "pub fn rust_func() {}" > "$dir/lib.rs"
    echo "pub fn python_func():" > "$dir/lib.py"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig --type rs 'pub fn' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "lib.rs" && assert_not_contains "lib.py"'
    run_claude_test "C08" "Type filter" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C09 — Index build + status
# ---------------------------------------------------------------------------
test_C09() {
    local dir; dir=$(new_project)
    echo "content1" > "$dir/file1.txt"
    echo "content2" > "$dir/file2.txt"
    echo "content3" > "$dir/file3.txt"

    local prompt="Run \`ig index $dir\` and then \`ig status $dir\` via the Bash tool. Show the exact raw output of both commands."
    local assert='echo "$result" | grep -qiE "(index|file|[0-9])"'
    run_claude_test "C09" "Index build + status" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C10 — Overlay (new file detection)
# ---------------------------------------------------------------------------
test_C10() {
    local dir; dir=$(new_project)
    echo "original" > "$dir/old.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    echo "newcontent_unique_marker" > "$dir/fresh.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig 'newcontent_unique_marker' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "fresh.txt"'
    run_claude_test "C10" "Overlay detects new files" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C11 — Error: bad regex
# ---------------------------------------------------------------------------
test_C11() {
    local dir; dir=$(new_project)
    echo "dummy" > "$dir/x.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Run \`ig '[invalid' $dir\` via the Bash tool and tell me what happens. Show the exact raw output of the ig command."
    local assert='echo "$result" | grep -qiE "(error|invalid|unclosed|parse)"'
    run_claude_test "C11" "Error on bad regex" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C12 — Error: empty pattern
# ---------------------------------------------------------------------------
test_C12() {
    local dir; dir=$(new_project)
    echo "dummy" > "$dir/x.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Run \`ig '' $dir\` via the Bash tool and tell me what happens. Show the exact raw output of the ig command."
    local assert='echo "$result" | grep -qiE "(empty|error|pattern|usage|required)"'
    run_claude_test "C12" "Error on empty pattern" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C13 — Max file size
# ---------------------------------------------------------------------------
test_C13() {
    local dir; dir=$(new_project)
    printf 'x_small' > "$dir/small.txt"
    python3 -c "print('x_large ' * 50)" > "$dir/large.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig --max-file-size 50 'x_' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains "small.txt" && assert_not_contains "large.txt"'
    run_claude_test "C13" "Max file size filter" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C14 — Context lines
# ---------------------------------------------------------------------------
test_C14() {
    local dir; dir=$(new_project)
    printf 'line_above\nneedle_target\nline_below\n' > "$dir/ctx.txt"
    "$IG" index "$dir" >/dev/null 2>&1

    local prompt="Use \`ig -C 1 'needle_target' $dir\` via the Bash tool. Show the exact raw output of the ig command."
    local assert='assert_contains_all "line_above" "needle_target" "line_below"'
    run_claude_test "C14" "Context lines" "$prompt" "$assert"
}

# ---------------------------------------------------------------------------
# C15 — Cross-project search
# ---------------------------------------------------------------------------
test_C15() {
    local dir1; dir1=$(new_project)
    local dir2; dir2=$(new_project)
    echo "alpha_marker here" > "$dir1/a.txt"
    echo "beta_marker here" > "$dir2/b.txt"
    "$IG" index "$dir1" >/dev/null 2>&1
    "$IG" index "$dir2" >/dev/null 2>&1

    local prompt="Use the Bash tool to run \`ig 'alpha_marker' $dir1\` and then \`ig 'beta_marker' $dir2\`. Show the exact raw output of both ig commands."
    local assert='assert_contains_all "alpha_marker" "beta_marker"'
    run_claude_test "C15" "Cross-project search" "$prompt" "$assert"
}

# ===========================================================================
# Run all tests
# ===========================================================================

echo -e "${BOLD}Running tests...${NC}\n"

test_C01
test_C02
test_C03
test_C04
test_C05
test_C06
test_C07
test_C08
test_C09
test_C10
test_C11
test_C12
test_C13
test_C14
test_C15

# ===========================================================================
# Summary
# ===========================================================================

TOTAL=$((PASS + FAIL + SKIP))

echo ""
echo -e "${BOLD}=== Summary ===${NC}"
echo -e "  Total:   $TOTAL"
echo -e "  ${GREEN}Passed:  $PASS${NC}"
echo -e "  ${RED}Failed:  $FAIL${NC}"
echo -e "  ${YELLOW}Skipped: $SKIP${NC}"

if [[ ${#ERRORS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${RED}Failures:${NC}"
    for err in "${ERRORS[@]}"; do
        echo -e "  - $err"
    done
fi

echo ""
if [[ $FAIL -eq 0 ]]; then
    echo -e "${GREEN}${BOLD}All tests passed.${NC}"
    exit 0
else
    echo -e "${RED}${BOLD}$FAIL test(s) failed.${NC}"
    exit 1
fi
