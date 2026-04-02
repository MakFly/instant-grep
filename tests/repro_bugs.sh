#!/bin/bash
# Reproduction script for ig bugs found during debug-detective analysis
# Run from project root: bash tests/repro_bugs.sh

set +e
IG="./target/release/ig"
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

PASS=0
FAIL=0

check() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$actual" == *"$expected"* ]]; then
        echo "  PASS: $name"
        ((PASS++))
    else
        echo "  FAIL: $name (expected: '$expected', got: '${actual:0:200}')"
        ((FAIL++))
    fi
}

echo "=== H1: New files not detected in non-git projects ==="
mkdir -p "$TMPDIR/h1"
echo "function alpha() { return 1; }" > "$TMPDIR/h1/a.txt"
echo "function beta() { return 2; }" > "$TMPDIR/h1/b.txt"
$IG index "$TMPDIR/h1" > /dev/null 2>&1

echo "function gamma() { return 3; }" > "$TMPDIR/h1/c.txt"
$IG index "$TMPDIR/h1" > /dev/null 2>&1

RESULT=$($IG "gamma" "$TMPDIR/h1" 2>/dev/null)
check "H1: new file detected after re-index" "gamma" "$RESULT"

STATUS=$($IG status "$TMPDIR/h1" 2>&1)
check "H1: file count includes new file (3)" "3 files" "$STATUS"

echo ""
echo "=== H2: External path resolves to wrong index root ==="
mkdir -p "$TMPDIR/h2/subdir"
echo "searchterm_unique_42" > "$TMPDIR/h2/root.txt"
echo "searchterm_unique_42" > "$TMPDIR/h2/subdir/nested.txt"
$IG index "$TMPDIR/h2" > /dev/null 2>&1

RESULT=$($IG "searchterm_unique_42" "$TMPDIR/h2/subdir/" 2>/dev/null)
check "H2: subdirectory search finds content" "searchterm_unique_42" "$RESULT"

echo ""
echo "=== H3: Hook deny exit code ==="
$IG rewrite 'rm -rf .' > /dev/null 2>&1
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 2 ]]; then
    echo "  PASS: H3 ig rewrite exits 2 for rm -rf ."
    ((PASS++))
else
    echo "  FAIL: H3 ig rewrite exits $EXIT_CODE instead of 2"
    ((FAIL++))
fi

echo '{"tool_input":{"command":"rm -rf ."}}' | bash hooks/ig-rewrite.sh > /dev/null 2>&1
HOOK_EXIT=$?
if [[ $HOOK_EXIT -eq 2 ]]; then
    echo "  PASS: H3 hook passes deny (exit 2)"
    ((PASS++))
else
    echo "  FAIL: H3 hook swallows deny (exit $HOOK_EXIT instead of 2)"
    ((FAIL++))
fi

echo ""
echo "=== H4: Empty pattern ==="
RESULT=$($IG "" 2>/dev/null | wc -c | tr -d ' ')
if [[ "$RESULT" -gt 10000 ]]; then
    echo "  FAIL: H4 empty pattern produces $RESULT bytes (should be rejected)"
    ((FAIL++))
else
    echo "  PASS: H4 empty pattern is handled"
    ((PASS++))
fi

echo ""
echo "================================"
echo "Results: $PASS passed, $FAIL failed"
exit $FAIL
