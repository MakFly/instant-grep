#!/usr/bin/env bash
# ig integration tests part 2: T071-T140
# INDEX_INTEGRITY, REWRITE, HOOK, GIT_PROXY

set -o pipefail

IG="${IG:-./target/release/ig}"
PASS=0
FAIL=0
SKIP=0
ERRORS=()
FILTER="${1:-}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

MASTER_TMP=$(mktemp -d)
trap "rm -rf $MASTER_TMP" EXIT

new_project() {
    local dir="$MASTER_TMP/proj_$$_$RANDOM"
    mkdir -p "$dir"
    (cd "$dir" && git init -q && git commit --allow-empty -m "init" -q)
    echo "$dir"
}

new_project_nogit() {
    local dir="$MASTER_TMP/proj_nogit_$$_$RANDOM"
    mkdir -p "$dir"
    echo "$dir"
}

index_project() {
    $IG index "$1" >/dev/null 2>&1
}

run_test() {
    local id="$1" category="$2" desc="$3"
    if [[ -n "$FILTER" ]] && ! echo "$id $category $desc" | grep -qi "$FILTER"; then
        return
    fi
    if eval "$4" 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC} $id: $category — $desc"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $id: $category — $desc"
        ((FAIL++))
        ERRORS+=("$id: $desc")
    fi
}

echo -e "${BOLD}ig integration tests part 2 (T071-T140)${NC}"
echo "========================================"

# ── INDEX_INTEGRITY ──────────────────────────────────────────────────────────

echo -e "\n${BOLD}INDEX_INTEGRITY${NC}"

run_test "T071" "INDEX_INTEGRITY" "modified file content updated after rebuild" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn old_function() {}" > "$P/src/main.rs"
    (cd "$P" && git add -A && git commit -m "add" -q)
    index_project "$P"
    echo "fn new_function() {}" > "$P/src/main.rs"
    (cd "$P" && git add -A && git commit -m "modify" -q)
    $IG index "$P" >/dev/null 2>&1
    NEW=$($IG "new_function" "$P" 2>/dev/null)
    OLD=$($IG "old_function" "$P" 2>/dev/null)
    [[ -n "$NEW" ]] && [[ -z "$OLD" ]]
'

run_test "T072" "INDEX_INTEGRITY" "overlay files exist after incremental update" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn first() {}" > "$P/src/main.rs"
    index_project "$P"
    echo "fn second() {}" >> "$P/src/main.rs"
    $IG index "$P" >/dev/null 2>&1
    [[ -f "$P/.ig/overlay.bin" ]] && [[ -f "$P/.ig/overlay_lex.bin" ]]
'

run_test "T073" "INDEX_INTEGRITY" "tombstone invalidates deleted doc" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn helper_xyz() {}" > "$P/src/lib.rs"
    echo "fn main() {}" > "$P/src/main.rs"
    index_project "$P"
    rm "$P/src/lib.rs"
    $IG index "$P" >/dev/null 2>&1
    OUT=$($IG "helper_xyz" "$P" 2>/dev/null)
    [[ -z "$OUT" ]]
'

run_test "T074" "INDEX_INTEGRITY" "ig status includes overlay count" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn first() {}" > "$P/src/main.rs"
    (cd "$P" && git add -A && git commit -m "add" -q)
    index_project "$P"
    echo "fn second() {}" >> "$P/src/main.rs"
    (cd "$P" && git add -A && git commit -m "modify" -q)
    $IG index "$P" >/dev/null 2>&1
    OUT=$($IG status "$P" 2>&1)
    echo "$OUT" | grep -q "files"
'

run_test "T075" "INDEX_INTEGRITY" "corrupted metadata.bin → graceful fallback (rebuild)" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() { println!(\"hello\"); }" > "$P/src/main.rs"
    index_project "$P"
    echo "garbage_data_xyz" > "$P/.ig/metadata.bin"
    OUT=$($IG "main" "$P" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]]
'

run_test "T076" "INDEX_INTEGRITY" "corrupted postings.bin → no panic" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() {}" > "$P/src/main.rs"
    index_project "$P"
    truncate -s 0 "$P/.ig/postings.bin"
    $IG "main" "$P" >/dev/null 2>&1
    EXIT=$?
    [[ $EXIT -ne 101 ]]
'

run_test "T077" "INDEX_INTEGRITY" "missing lexicon.bin → graceful error (exit non-zero)" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() {}" > "$P/src/main.rs"
    index_project "$P"
    rm "$P/.ig/lexicon.bin"
    $IG "main" "$P" >/dev/null 2>&1
    EXIT=$?
    [[ $EXIT -ne 0 ]]
'

run_test "T078" "INDEX_INTEGRITY" "version mismatch triggers rebuild on search" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() { println!(\"hello\"); }" > "$P/src/main.rs"
    index_project "$P"
    python3 -c "import struct; open(\"$P/.ig/metadata.bin\", \"wb\").write(struct.pack(\"<I\", 999))" 2>/dev/null
    OUT=$($IG "main" "$P" 2>/dev/null)
    [[ -n "$OUT" ]]
'

run_test "T079" "INDEX_INTEGRITY" "file count mismatch triggers rebuild" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() {}" > "$P/src/main.rs"
    (cd "$P" && git add -A && git commit -m "add" -q)
    index_project "$P"
    echo "fn extra_xyz() {}" > "$P/src/extra.rs"
    (cd "$P" && git add -A && git commit -m "add extra" -q)
    $IG index "$P" >/dev/null 2>&1
    OUT=$($IG "extra_xyz" "$P" 2>/dev/null)
    [[ -n "$OUT" ]]
'

run_test "T080" "INDEX_INTEGRITY" "ig index on already up-to-date project says up to date" '
    P=$(new_project)
    mkdir -p "$P/src"
    echo "fn main() {}" > "$P/src/main.rs"
    index_project "$P"
    OUT=$($IG index "$P" 2>&1)
    echo "$OUT" | grep -qi "up to date"
'

# ── REWRITE ───────────────────────────────────────────────────────────────────

echo -e "\n${BOLD}REWRITE${NC}"

run_test "T081" "REWRITE" "cat file → ig read file" '
    OUT=$($IG rewrite "cat src/main.rs" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig read src/main.rs" ]]
'

run_test "T082" "REWRITE" "cat with flags → passthrough" '
    $IG rewrite "cat -n src/main.rs" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T083" "REWRITE" "head file → ig read file" '
    OUT=$($IG rewrite "head src/main.rs" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig read src/main.rs" ]]
'

run_test "T084" "REWRITE" "head -50 file → ig read file" '
    OUT=$($IG rewrite "head -50 src/main.rs" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig read src/main.rs" ]]
'

run_test "T085" "REWRITE" "tail file → ig read file" '
    OUT=$($IG rewrite "tail src/main.rs" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig read src/main.rs" ]]
'

run_test "T086" "REWRITE" "grep -r pattern dir → ig \"pattern\" dir" '
    OUT=$($IG rewrite "grep -r useState src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig \"useState\" src/" ]]
'

run_test "T087" "REWRITE" "grep -ri pattern . → ig -i \"pattern\"" '
    OUT=$($IG rewrite "grep -ri pattern ." 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig -i \"pattern\"" ]]
'

run_test "T088" "REWRITE" "grep non-recursive → passthrough" '
    $IG rewrite "grep pattern file.txt" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T089" "REWRITE" "grep -e explicit pattern" '
    OUT=$($IG rewrite "grep -r -e pattern src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig \"pattern\" src/" ]]
'

run_test "T090" "REWRITE" "rg pattern dir → ig \"pattern\" dir" '
    OUT=$($IG rewrite "rg useState src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig \"useState\" src/" ]]
'

run_test "T091" "REWRITE" "rg -i → ig -i" '
    OUT=$($IG rewrite "rg -i pattern" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig -i \"pattern\"" ]]
'

run_test "T092" "REWRITE" "rg -t ts → ig --type ts" '
    OUT=$($IG rewrite "rg -t ts pattern" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig --type ts \"pattern\"" ]]
'

run_test "T093" "REWRITE" "tree → cat .ig/tree.txt || ig ls" '
    OUT=$($IG rewrite "tree" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "cat .ig/tree.txt 2>/dev/null || ig ls" ]]
'

run_test "T094" "REWRITE" "tree with flags → same rewrite" '
    OUT=$($IG rewrite "tree -L 3 -I node_modules" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "cat .ig/tree.txt 2>/dev/null || ig ls" ]]
'

run_test "T095" "REWRITE" "find -name \"*.ts\" → ig files --glob" '
    OUT=$($IG rewrite "find . -name \"*.ts\"" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig files --glob \"*.ts\"" ]]
'

run_test "T096" "REWRITE" "find -type f -name \"*.rs\" → ig files --glob" '
    OUT=$($IG rewrite "find . -type f -name \"*.rs\"" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig files --glob \"*.rs\"" ]]
'

run_test "T097" "REWRITE" "find -type d → passthrough" '
    $IG rewrite "find . -type d -name src" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T098" "REWRITE" "find with -exec → passthrough" '
    $IG rewrite "find . -name \"*.ts\" -exec rm {} ;" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T099" "REWRITE" "ls → ig ls" '
    OUT=$($IG rewrite "ls" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig ls" ]]
'

run_test "T100" "REWRITE" "ls src/ → ig ls src/" '
    OUT=$($IG rewrite "ls src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig ls src/" ]]
'

run_test "T101" "REWRITE" "ls -la src/ → ig ls src/" '
    OUT=$($IG rewrite "ls -la src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig ls src/" ]]
'

run_test "T102" "REWRITE" "ls multiple paths → passthrough" '
    $IG rewrite "ls src/ tests/" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T103" "REWRITE" "git status → ig git status" '
    OUT=$($IG rewrite "git status" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig git status" ]]
'

run_test "T104" "REWRITE" "git log → ig git log" '
    OUT=$($IG rewrite "git log" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig git log" ]]
'

run_test "T105" "REWRITE" "git diff → ig git diff" '
    OUT=$($IG rewrite "git diff" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig git diff" ]]
'

run_test "T106" "REWRITE" "git show HEAD → ig git show HEAD" '
    OUT=$($IG rewrite "git show HEAD" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig git show HEAD" ]]
'

run_test "T107" "REWRITE" "git branch → ig git branch" '
    OUT=$($IG rewrite "git branch" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig git branch" ]]
'

run_test "T108" "REWRITE" "git commit → passthrough" '
    $IG rewrite "git commit -m test" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T109" "REWRITE" "git checkout → passthrough" '
    $IG rewrite "git checkout main" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T110" "REWRITE" "DENY git reset --hard → exit 2" '
    $IG rewrite "git reset --hard" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T111" "REWRITE" "DENY git reset --hard HEAD~1 → exit 2" '
    $IG rewrite "git reset --hard HEAD~1" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T112" "REWRITE" "DENY git clean -f → exit 2" '
    $IG rewrite "git clean -f" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T113" "REWRITE" "DENY git clean -fd → exit 2" '
    $IG rewrite "git clean -fd" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T114" "REWRITE" "DENY rm -rf / → exit 2" '
    $IG rewrite "rm -rf /" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T115" "REWRITE" "DENY rm -rf . → exit 2" '
    $IG rewrite "rm -rf ." >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T116" "REWRITE" "DENY rm -rf ~/ → exit 2" '
    $IG rewrite "rm -rf ~/" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T117" "REWRITE" "DENY rm -rf ./ → exit 2" '
    $IG rewrite "rm -rf ./" >/dev/null 2>/dev/null; [[ $? -eq 2 ]]
'

run_test "T118" "REWRITE" "safe rm -rf specific path → passthrough (exit 1)" '
    $IG rewrite "rm -rf /tmp/myproject" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T119" "REWRITE" "ASK git push --force → exit 3" '
    $IG rewrite "git push --force" >/dev/null 2>/dev/null; [[ $? -eq 3 ]]
'

run_test "T120" "REWRITE" "ASK git push -f → exit 3" '
    $IG rewrite "git push -f" >/dev/null 2>/dev/null; [[ $? -eq 3 ]]
'

run_test "T121" "REWRITE" "ASK git push --force-with-lease → exit 3" '
    $IG rewrite "git push --force-with-lease" >/dev/null 2>/dev/null; [[ $? -eq 3 ]]
'

run_test "T122" "REWRITE" "pipe → passthrough" '
    $IG rewrite "echo hello | grep hello" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T123" "REWRITE" "&& → passthrough" '
    $IG rewrite "cat file && echo done" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T124" "REWRITE" "|| → passthrough" '
    $IG rewrite "cat file || echo fail" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T125" "REWRITE" "semicolon → passthrough" '
    $IG rewrite "cat file; ls" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

run_test "T126" "REWRITE" "quoted pattern with spaces" '
    OUT=$($IG rewrite "grep -r \"hello world\" src/" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig \"hello world\" src/" ]]
'

run_test "T127" "REWRITE" "single-quoted filename" '
    OUT=$($IG rewrite "cat '"'"'my file.rs'"'"'" 2>/dev/null); EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ "$OUT" == "ig read my file.rs" ]]
'

run_test "T128" "REWRITE" "empty command → passthrough" '
    $IG rewrite "" >/dev/null 2>/dev/null; [[ $? -eq 1 ]]
'

# ── HOOK ─────────────────────────────────────────────────────────────────────

echo -e "\n${BOLD}HOOK${NC}"

HOOK_REWRITE="hooks/ig-rewrite.sh"
HOOK_PREFER="hooks/prefer-ig.sh"

run_test "T129" "HOOK" "ig-rewrite.sh cat triggers rewrite JSON" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"cat src/main.rs\"}}" | bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]] && echo "$OUT" | grep -q "ig read"
'

run_test "T130" "HOOK" "ig-rewrite.sh denied cmd exits 2" '
    echo "{\"tool_input\":{\"command\":\"git reset --hard\"}}" | bash "$HOOK_REWRITE" >/dev/null 2>/dev/null
    [[ $? -eq 2 ]]
'

run_test "T131" "HOOK" "ig-rewrite.sh passthrough exits 0 no output" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"cargo test\"}}" | bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ -z "$OUT" ]]
'

run_test "T132" "HOOK" "ig-rewrite.sh ask (push --force) returns JSON without permissionDecision=allow" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"git push --force\"}}" | bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]] && echo "$OUT" | grep -q "hookSpecificOutput" && ! echo "$OUT" | grep -q "\"permissionDecision\":\"allow\""
'

run_test "T133" "HOOK" "ig-rewrite.sh missing jq → exit 0" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"cat file\"}}" | env PATH="/usr/bin:/bin" bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]]
'

run_test "T134" "HOOK" "ig-rewrite.sh missing ig → exit 0" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"cat file\"}}" | env HOME="/nonexistent" PATH="/usr/bin:/bin" bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]]
'

run_test "T135" "HOOK" "ig-rewrite.sh empty command → exit 0" '
    OUT=$(echo "{\"tool_input\":{\"command\":\"\"}}" | bash "$HOOK_REWRITE" 2>/dev/null)
    EXIT=$?
    [[ $EXIT -eq 0 ]] && [[ -z "$OUT" ]]
'

run_test "T136" "HOOK" "prefer-ig.sh rg blocked" '
    CLAUDE_BASH_COMMAND="rg useState src/" bash "$HOOK_PREFER" >/dev/null 2>/dev/null
    [[ $? -eq 2 ]]
'

run_test "T137" "HOOK" "prefer-ig.sh grep -r blocked" '
    CLAUDE_BASH_COMMAND="grep -r pattern src/" bash "$HOOK_PREFER" >/dev/null 2>/dev/null
    [[ $? -eq 2 ]]
'

run_test "T138" "HOOK" "prefer-ig.sh piped grep allowed" '
    CLAUDE_BASH_COMMAND="echo hello | grep hello" bash "$HOOK_PREFER" >/dev/null 2>/dev/null
    [[ $? -eq 0 ]]
'

run_test "T139" "HOOK" "prefer-ig.sh find -name blocked" '
    CLAUDE_BASH_COMMAND="find . -name \"*.rs\"" bash "$HOOK_PREFER" >/dev/null 2>/dev/null
    [[ $? -eq 2 ]]
'

run_test "T140" "HOOK" "prefer-ig.sh find -maxdepth 1 allowed" '
    CLAUDE_BASH_COMMAND="find . -maxdepth 1 -name \"*.rs\"" bash "$HOOK_PREFER" >/dev/null 2>/dev/null
    [[ $? -eq 0 ]]
'

# ── SUMMARY ──────────────────────────────────────────────────────────────────

echo ""
echo "========================================"
TOTAL=$((PASS + FAIL + SKIP))
echo -e "${BOLD}Results: ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}, ${YELLOW}$SKIP skipped${NC} / $TOTAL total"

if [[ ${#ERRORS[@]} -gt 0 ]]; then
    echo -e "\n${RED}Failed tests:${NC}"
    for e in "${ERRORS[@]}"; do
        echo "  - $e"
    done
fi

[[ $FAIL -eq 0 ]]
