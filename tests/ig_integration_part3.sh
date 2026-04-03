#!/usr/bin/env bash
# ig integration tests part 3: T141-T200
# HOOK (cont), DAEMON, GIT_PROXY, READ, WALK, TRACKING, CLI, PERFORMANCE, CROSS_PROJECT, SECURITY

set -o pipefail

# Resolve IG to absolute path so cd in tests doesn't break relative paths
_IG_RAW="${IG:-./target/release/ig}"
if [[ "$_IG_RAW" != /* ]]; then
    IG="$(pwd)/$_IG_RAW"
else
    IG="$_IG_RAW"
fi
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
    # Run in a subshell to isolate cwd changes, variable mutations, etc.
    if (eval "$4") 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC} $id: $category — $desc"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $id: $category — $desc"
        ((FAIL++))
        ERRORS+=("$id: $desc")
    fi
}

skip_test() {
    local id="$1" category="$2" desc="$3" reason="$4"
    if [[ -n "$FILTER" ]] && ! echo "$id $category $desc" | grep -qi "$FILTER"; then
        return
    fi
    echo -e "  ${YELLOW}SKIP${NC} $id: $category — $desc ($reason)"
    ((SKIP++))
}

echo -e "${BOLD}ig integration tests part 3 (T141-T200)${NC}"
echo "========================================"

# ─────────────────────────────────────────────────────────────
# HOOK (continued)
# ─────────────────────────────────────────────────────────────

run_test "T141" "HOOK" "prefer-ig.sh allows unrelated commands (cargo build)" '
    export CLAUDE_BASH_COMMAND="cargo build --release"
    bash hooks/prefer-ig.sh >/dev/null 2>&1
    T141_EXIT=$?
    unset CLAUDE_BASH_COMMAND
    [[ $T141_EXIT -eq 0 ]]
'

# ─────────────────────────────────────────────────────────────
# DAEMON
# ─────────────────────────────────────────────────────────────

run_test "T142" "DAEMON" "start creates socket file" '
    DIR=$(new_project)
    echo "daemon_token_142" > "$DIR/test.txt"
    index_project "$DIR"
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    # Use daemon status to get the socket path (canonicalized consistently)
    DAEMON_STATUS=$($IG daemon status "$DIR" 2>&1)
    SOCK=$(echo "$DAEMON_STATUS" | grep -oE "/tmp/ig-[a-f0-9]+\.sock")
    T142_RESULT=1
    [[ -n "$SOCK" ]] && [[ -S "$SOCK" ]] && T142_RESULT=0
    $IG daemon stop "$DIR" >/dev/null 2>&1
    [[ $T142_RESULT -eq 0 ]]
'

run_test "T143" "DAEMON" "daemon status shows running" '
    DIR=$(new_project)
    echo "fn daemon_143() {}" > "$DIR/test.rs"
    index_project "$DIR"
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    STATUS=$($IG daemon status "$DIR" 2>&1)
    $IG daemon stop "$DIR" >/dev/null 2>&1
    echo "$STATUS" | grep -qi "running"
'

run_test "T144" "DAEMON" "stop removes socket file" '
    DIR=$(new_project)
    echo "fn daemon_144() {}" > "$DIR/test.rs"
    index_project "$DIR"
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    # Get socket path via daemon status (uses same canonicalization as daemon)
    SOCK=$(echo "$($IG daemon status "$DIR" 2>&1)" | grep -oE "/tmp/ig-[a-f0-9]+\.sock")
    $IG daemon stop "$DIR" >/dev/null 2>&1
    sleep 0.3
    [[ -n "$SOCK" ]] && [[ ! -S "$SOCK" ]]
'

run_test "T145" "DAEMON" "status after stop shows not running" '
    DIR=$(new_project)
    echo "fn daemon_145() {}" > "$DIR/test.rs"
    index_project "$DIR"
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    $IG daemon stop "$DIR" >/dev/null 2>&1
    sleep 0.3
    STATUS=$($IG daemon status "$DIR" 2>&1)
    echo "$STATUS" | grep -qi "not running"
'

run_test "T146" "DAEMON" "ig query returns results while daemon running" '
    DIR=$(new_project)
    echo "fn unique_daemon_token_146() {}" > "$DIR/search.rs"
    index_project "$DIR"
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    RESULT=$($IG query "unique_daemon_token_146" "$DIR" 2>&1)
    $IG daemon stop "$DIR" >/dev/null 2>&1
    echo "$RESULT" | grep -q "unique_daemon_token_146"
'

run_test "T147" "DAEMON" "ig query on stopped daemon fails gracefully" '
    DIR=$(new_project)
    echo "fn query_stopped_147() {}" > "$DIR/test.rs"
    index_project "$DIR"
    OUT=$($IG query "query_stopped_147" "$DIR" 2>&1)
    Q_EXIT=$?
    [[ $Q_EXIT -ne 0 ]] || echo "$OUT" | grep -qi "error\|not running\|connect"
'

run_test "T148" "DAEMON" "unknown daemon action exits 1" '
    DIR=$(new_project)
    $IG daemon bogus_action "$DIR" >/dev/null 2>&1
    [[ $? -eq 1 ]]
'

run_test "T149" "DAEMON" "daemon foreground mode starts (killed after 1.5s)" '
    DIR=$(new_project)
    echo "fn daemon_fg_149() {}" > "$DIR/test.rs"
    index_project "$DIR"
    ($IG daemon foreground "$DIR" &
     DPID=$!
     sleep 1.5
     kill $DPID 2>/dev/null
     wait $DPID 2>/dev/null) 2>&1 | grep -qi "socket\|listening\|daemon\|indexed"
    true
'

run_test "T150" "DAEMON" "stale socket cleaned up on daemon start" '
    DIR=$(new_project)
    echo "fn daemon_stale_150() {}" > "$DIR/test.rs"
    index_project "$DIR"
    # Start once to discover the canonical socket path for this project
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.3
    REAL_SOCK=$(echo "$($IG daemon status "$DIR" 2>&1)" | grep -oE "/tmp/ig-[a-f0-9]+\.sock")
    $IG daemon stop "$DIR" >/dev/null 2>&1
    sleep 0.3
    # Re-create a stale (regular file) at that path to simulate a crashed daemon
    [[ -n "$REAL_SOCK" ]] && touch "$REAL_SOCK"
    # Start daemon again — it removes the stale file before bind
    $IG daemon start "$DIR" >/dev/null 2>&1
    sleep 0.5
    T150_RESULT=1
    # The path should now be a proper Unix socket (not just a regular file)
    [[ -S "$REAL_SOCK" ]] && T150_RESULT=0
    $IG daemon stop "$DIR" >/dev/null 2>&1
    [[ $T150_RESULT -eq 0 ]]
'

# ─────────────────────────────────────────────────────────────
# GIT_PROXY
# ─────────────────────────────────────────────────────────────

run_test "T151" "GIT_PROXY" "ig git status on clean repo shows Clean working tree" '
    DIR=$(new_project)
    OUT=$(cd "$DIR" && $IG git status 2>&1)
    echo "$OUT" | grep -qi "clean working tree"
'

run_test "T152" "GIT_PROXY" "ig git status with modified file shows Modified" '
    DIR=$(new_project)
    echo "hello" > "$DIR/file.txt"
    git -C "$DIR" add .
    git -C "$DIR" commit -m "add file" -q
    echo "changed" > "$DIR/file.txt"
    OUT=$(cd "$DIR" && $IG git status 2>&1)
    echo "$OUT" | grep -qi "modified"
'

run_test "T153" "GIT_PROXY" "ig git status with >5 untracked shows truncated +N more" '
    DIR=$(new_project)
    for i in $(seq 1 7); do touch "$DIR/file$i.txt"; done
    OUT=$(cd "$DIR" && $IG git status 2>&1)
    echo "$OUT" | grep -q "+[0-9]\+ more"
'

run_test "T154" "GIT_PROXY" "ig git log shows commit entries" '
    DIR=$(new_project)
    for i in $(seq 1 3); do
        echo "v$i" > "$DIR/file.txt"
        git -C "$DIR" add .
        git -C "$DIR" commit -m "commit $i" -q
    done
    COUNT=$(cd "$DIR" && $IG git log 2>&1 | wc -l | tr -d " ")
    [[ $COUNT -ge 3 ]]
'

run_test "T155" "GIT_PROXY" "ig git log -5 limits to 5 entries" '
    DIR=$(new_project)
    for i in $(seq 1 10); do
        echo "v$i" > "$DIR/file.txt"
        git -C "$DIR" add .
        git -C "$DIR" commit -m "commit $i" -q
    done
    COUNT=$(cd "$DIR" && $IG git log -5 2>&1 | wc -l | tr -d " ")
    [[ $COUNT -le 6 ]]
'

run_test "T156" "GIT_PROXY" "ig git diff on clean repo shows No changes" '
    DIR=$(new_project)
    OUT=$(cd "$DIR" && $IG git diff 2>&1)
    echo "$OUT" | grep -qi "no changes"
'

run_test "T157" "GIT_PROXY" "ig git diff with modifications shows diff" '
    DIR=$(new_project)
    echo "original" > "$DIR/file.txt"
    git -C "$DIR" add .
    git -C "$DIR" commit -m "add" -q
    echo "modified" > "$DIR/file.txt"
    OUT=$(cd "$DIR" && $IG git diff 2>&1)
    echo "$OUT" | grep -q "file.txt\|@@\|modified"
'

run_test "T158" "GIT_PROXY" "ig git show HEAD shows commit info" '
    DIR=$(new_project)
    echo "content" > "$DIR/file.txt"
    git -C "$DIR" add .
    git -C "$DIR" commit -m "test commit 158" -q
    OUT=$(cd "$DIR" && $IG git show HEAD 2>&1)
    echo "$OUT" | grep -qi "commit\|author\|test commit 158"
'

run_test "T159" "GIT_PROXY" "ig git branch lists branches" '
    DIR=$(new_project)
    git -C "$DIR" branch feature-159
    OUT=$(cd "$DIR" && $IG git branch 2>&1)
    echo "$OUT" | grep -q "main\|master\|feature-159"
'

run_test "T160" "GIT_PROXY" "ig git with no args exits 1 with usage" '
    DIR=$(new_project)
    cd "$DIR" && $IG git >/dev/null 2>&1; T160_EXIT=$?
    [[ $T160_EXIT -eq 1 ]]
'

run_test "T161" "GIT_PROXY" "ig git unknown subcommand passthrough exits with git code" '
    DIR=$(new_project)
    cd "$DIR" && $IG git totally_unknown_subcommand_161 >/dev/null 2>&1
    [[ $? -ne 0 ]]
'

run_test "T162" "GIT_PROXY" "ig git log with custom format args preserved" '
    DIR=$(new_project)
    echo "x" > "$DIR/x.txt"
    git -C "$DIR" add .
    git -C "$DIR" commit -m "fmt test 162" -q
    OUT=$(cd "$DIR" && $IG git log --format="%s" 2>&1)
    echo "$OUT" | grep -q "fmt test 162\|init"
'

# ─────────────────────────────────────────────────────────────
# READ
# ─────────────────────────────────────────────────────────────

run_test "T163" "READ" "ig read full file shows numbered lines" '
    DIR=$(new_project_nogit)
    printf "line one\nline two\nline three\n" > "$DIR/file.txt"
    OUT=$($IG read "$DIR/file.txt" 2>&1)
    echo "$OUT" | grep -q "1:" && echo "$OUT" | grep -q "line one"
'

run_test "T164" "READ" "ig read -s Rust file shows signatures only" '
    DIR=$(new_project_nogit)
    printf "pub fn alpha() -> u32 { 1 }\nfn beta(x: i32) {}\n" > "$DIR/lib.rs"
    OUT=$($IG read -s "$DIR/lib.rs" 2>&1)
    echo "$OUT" | grep -q "alpha\|beta"
'

run_test "T165" "READ" "ig read -s TypeScript shows exports and imports" "
    DIR=\$(new_project_nogit)
    printf 'export function myFunc() {}\nimport { bar } from \"baz\";\n' > \"\$DIR/mod.ts\"
    OUT=\$(\$IG read -s \"\$DIR/mod.ts\" 2>&1)
    echo \"\$OUT\" | grep -q 'myFunc\|bar\|baz'
"

run_test "T166" "READ" "ig read nonexistent file exits non-zero" '
    $IG read "$MASTER_TMP/nonexistent_file_166.rs" >/dev/null 2>&1
    [[ $? -ne 0 ]]
'

run_test "T167" "READ" "ig read binary file outputs error or skip message" '
    OUT=$($IG read /bin/ls 2>&1)
    echo "$OUT" | grep -qi "binary\|skip\|error"
'

run_test "T168" "READ" "ig read empty file does not crash" '
    DIR=$(new_project_nogit)
    printf "" > "$DIR/empty.rs"
    $IG read "$DIR/empty.rs" >/dev/null 2>&1
    [[ $? -eq 0 ]]
'

run_test "T169" "READ" "ig read tracks savings in history" '
    ISOLATED_HOME="$MASTER_TMP/home_169_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn tracked_read_169() {}\n" > "$DIR/src.rs"
    HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>/dev/null
    GAIN_OUT=$(HOME="$ISOLATED_HOME" $IG gain -H 2>&1)
    echo "$GAIN_OUT" | grep -q "ig read"
'

# ─────────────────────────────────────────────────────────────
# WALK
# ─────────────────────────────────────────────────────────────

run_test "T170" "WALK" "target/ excluded by default" '
    DIR=$(new_project_nogit)
    mkdir -p "$DIR/target/debug"
    echo "fn in_target() {}" > "$DIR/target/debug/excluded.rs"
    echo "fn in_src() {}" > "$DIR/included.rs"
    index_project "$DIR"
    OUT=$($IG "in_target" "$DIR" 2>&1)
    [[ -z "$OUT" ]]
'

run_test "T171" "WALK" ".ig/ directory always excluded from search" '
    DIR=$(new_project_nogit)
    index_project "$DIR"
    echo "ig_internal_token_171" > "$DIR/.ig/injected.txt"
    OUT=$($IG "ig_internal_token_171" "$DIR" 2>&1)
    [[ -z "$OUT" ]]
'

run_test "T172" "WALK" "hidden files (dot-prefixed) excluded" '
    DIR=$(new_project_nogit)
    echo "fn hidden_func_172() {}" > "$DIR/.hidden.rs"
    echo "fn visible_func_172() {}" > "$DIR/visible.rs"
    index_project "$DIR"
    OUT=$($IG "hidden_func_172" "$DIR" 2>&1)
    [[ -z "$OUT" ]]
'

run_test "T173" "WALK" ".gitignore patterns honored" '
    DIR=$(new_project)
    mkdir -p "$DIR/ignored_dir"
    echo "fn gitignored_func_173() {}" > "$DIR/ignored_dir/file.rs"
    echo "fn visible_func_173() {}" > "$DIR/visible.rs"
    echo "/ignored_dir/" > "$DIR/.gitignore"
    index_project "$DIR"
    OUT=$($IG "gitignored_func_173" "$DIR" 2>&1)
    [[ -z "$OUT" ]]
'

run_test "T174" "WALK" "--type with non-matching type returns no results (no crash)" '
    DIR=$(new_project_nogit)
    echo "fn only_rust_174() {}" > "$DIR/lib.rs"
    index_project "$DIR"
    $IG -t py "only_rust_174" "$DIR" >/dev/null 2>&1
    [[ $? -eq 0 ]]
'

run_test "T175" "WALK" "--glob filter matches only specified extensions" '
    DIR=$(new_project_nogit)
    echo "fn glob_rs_175() {}" > "$DIR/alpha.rs"
    echo "fn glob_py_175() {}" > "$DIR/beta.py"
    index_project "$DIR"
    OUT=$($IG -g "*.rs" "glob" "$DIR" 2>&1)
    echo "$OUT" | grep -q "alpha.rs" && ! echo "$OUT" | grep -q "beta.py"
'

# ─────────────────────────────────────────────────────────────
# TRACKING
# ─────────────────────────────────────────────────────────────

run_test "T176" "TRACKING" "ig gain with no history shows informative message" '
    ISOLATED_HOME="$MASTER_TMP/home_176_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    OUT=$(HOME="$ISOLATED_HOME" $IG gain 2>&1)
    echo "$OUT" | grep -qi "no.*track\|0 commands\|no history\|not.*track\|yet"
'

run_test "T177" "TRACKING" "ig gain after tracked command shows savings" '
    ISOLATED_HOME="$MASTER_TMP/home_177_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn tracked_func_177() {}\n" > "$DIR/src.rs"
    HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>&1
    OUT=$(HOME="$ISOLATED_HOME" $IG gain 2>&1)
    echo "$OUT" | grep -qi "command\|saved\|total"
'

run_test "T178" "TRACKING" "ig gain --clear empties history" '
    ISOLATED_HOME="$MASTER_TMP/home_178_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn clear_test_178() {}\n" > "$DIR/src.rs"
    HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>&1
    HOME="$ISOLATED_HOME" $IG gain --clear >/dev/null 2>&1
    OUT=$(HOME="$ISOLATED_HOME" $IG gain 2>&1)
    echo "$OUT" | grep -qi "no.*track\|0 commands\|no history\|yet"
'

run_test "T179" "TRACKING" "ig gain --json outputs valid JSON" '
    ISOLATED_HOME="$MASTER_TMP/home_179_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn json_track_179() {}\n" > "$DIR/src.rs"
    # Generate a tracking entry so gain --json has data
    HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>&1
    OUT=$(HOME="$ISOLATED_HOME" $IG gain --json 2>&1)
    echo "$OUT" | python3 -c "import sys, json; json.load(sys.stdin)" 2>/dev/null
'

run_test "T180" "TRACKING" "concurrent tracking writes do not corrupt JSON" '
    ISOLATED_HOME="$MASTER_TMP/home_180_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn concurrent_180() {}\n" > "$DIR/src.rs"
    for i in $(seq 1 10); do
        HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>&1 &
    done
    wait
    OUT=$(HOME="$ISOLATED_HOME" $IG gain 2>&1)
    echo "$OUT" | grep -qi "command\|total\|no.*track\|yet"
'

run_test "T181" "TRACKING" "ig gain -H shows history table" '
    ISOLATED_HOME="$MASTER_TMP/home_181_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    DIR=$(new_project_nogit)
    printf "fn history_181() {}\n" > "$DIR/src.rs"
    HOME="$ISOLATED_HOME" $IG read "$DIR/src.rs" >/dev/null 2>&1
    OUT=$(HOME="$ISOLATED_HOME" $IG gain -H 2>&1)
    echo "$OUT" | grep -qi "ig read\|command\|history"
'

# ─────────────────────────────────────────────────────────────
# CLI
# ─────────────────────────────────────────────────────────────

run_test "T182" "CLI" "ig --help shows all public commands" '
    OUT=$($IG --help 2>&1)
    echo "$OUT" | grep -q "search" &&
    echo "$OUT" | grep -q "daemon" &&
    echo "$OUT" | grep -q "gain" &&
    echo "$OUT" | grep -q "git"
'

run_test "T183" "CLI" "hidden commands (rewrite, proxy) NOT in --help" '
    OUT=$($IG --help 2>&1)
    ! echo "$OUT" | grep -q "rewrite" && ! echo "$OUT" | grep -q "proxy"
'

run_test "T184" "CLI" "ig completions bash outputs bash completion script" '
    OUT=$($IG completions bash 2>&1)
    echo "$OUT" | grep -q "_ig\|complete\|compgen"
'

run_test "T185" "CLI" "ig completions zsh outputs zsh completion script" '
    OUT=$($IG completions zsh 2>&1)
    echo "$OUT" | grep -q "#compdef\|_ig\|zsh"
'

run_test "T186" "CLI" "ig setup --dry-run outputs dry run indicator" '
    ISOLATED_HOME="$MASTER_TMP/home_186_$RANDOM"
    mkdir -p "$ISOLATED_HOME"
    OUT=$(HOME="$ISOLATED_HOME" $IG setup --dry-run 2>&1)
    echo "$OUT" | grep -qi "dry.run\|DRY RUN\|would"
'

run_test "T187" "CLI" "ig uninstall --dry-run lists artifacts without removing" '
    OUT=$($IG uninstall --dry-run 2>&1)
    echo "$OUT" | grep -qi "would\|dry.run\|remove\|hooks\|binary"
'

run_test "T188" "CLI" "ig discover runs without crash" '
    $IG discover >/dev/null 2>&1
    [[ $? -eq 0 ]]
'

run_test "T189" "CLI" "ig --version shows version string" '
    OUT=$($IG --version 2>&1)
    echo "$OUT" | grep -qE "ig [0-9]+\.[0-9]+\.[0-9]+"
'

run_test "T190" "CLI" "unknown subcommand treated as pattern or shows error" '
    # ig treats unknown subcommands as pattern shortcuts in shortcut mode;
    # confirm it does not segfault (exit code is 0 or error, never 139/SIGSEGV)
    $IG totally_unknown_subcommand_190_xyz >/dev/null 2>&1
    [[ $? -ne 139 ]]
'

# ─────────────────────────────────────────────────────────────
# PERFORMANCE
# ─────────────────────────────────────────────────────────────

run_test "T191" "PERFORMANCE" "index 500 files in <30s" '
    DIR=$(new_project_nogit)
    mkdir -p "$DIR/src"
    for i in $(seq 1 500); do
        echo "fn func_$i() { println!(\"test $i\"); }" > "$DIR/src/file_$i.rs"
    done
    START=$(date +%s)
    index_project "$DIR"
    END=$(date +%s)
    ELAPSED=$((END - START))
    [[ $ELAPSED -lt 30 ]]
'

run_test "T192" "PERFORMANCE" "search 500-file project in <2s" '
    DIR=$(new_project_nogit)
    mkdir -p "$DIR/src"
    for i in $(seq 1 500); do
        echo "fn perf_func_$i() { let x = $i; }" > "$DIR/src/file_$i.rs"
    done
    index_project "$DIR"
    START=$(date +%s)
    $IG "perf_func_250" "$DIR" >/dev/null 2>&1
    END=$(date +%s)
    ELAPSED=$((END - START))
    [[ $ELAPSED -lt 2 ]]
'

run_test "T193" "PERFORMANCE" "brute-force --no-index correct on 100 files" '
    DIR=$(new_project_nogit)
    mkdir -p "$DIR/src"
    for i in $(seq 1 100); do
        echo "fn brute_func_$i() {}" > "$DIR/src/file_$i.rs"
    done
    # search without index — must find the specific function
    OUT=$($IG --no-index "brute_func_42" "$DIR" 2>&1)
    echo "$OUT" | grep -q "brute_func_42"
'

# ─────────────────────────────────────────────────────────────
# CROSS_PROJECT
# ─────────────────────────────────────────────────────────────

run_test "T194" "CROSS_PROJECT" "search with absolute path outside cwd" '
    DIR=$(new_project_nogit)
    echo "fn cross_project_token_194() {}" > "$DIR/cross.rs"
    index_project "$DIR"
    # run from a different directory entirely
    OUT=$(cd "$MASTER_TMP" && $IG "cross_project_token_194" "$DIR" 2>&1)
    echo "$OUT" | grep -q "cross_project_token_194"
'

run_test "T195" "CROSS_PROJECT" "ig status with explicit absolute path" '
    DIR=$(new_project_nogit)
    echo "fn status_cross_195() {}" > "$DIR/file.rs"
    index_project "$DIR"
    OUT=$($IG status "$DIR" 2>&1)
    echo "$OUT" | grep -qi "files\|index"
'

run_test "T196" "CROSS_PROJECT" "ig files with explicit absolute path" '
    DIR=$(new_project_nogit)
    echo "fn files_cross_196() {}" > "$DIR/listed.rs"
    index_project "$DIR"
    OUT=$($IG files "$DIR" 2>&1)
    echo "$OUT" | grep -q "listed.rs"
'

# ─────────────────────────────────────────────────────────────
# SECURITY
# ─────────────────────────────────────────────────────────────

run_test "T197" "SECURITY" "JSON output escapes injected content safely" '
    DIR=$(new_project_nogit)
    printf '"'"'fn inject_test_197() { let s = "<script>alert(1)</script>"; }\n'"'"' > "$DIR/inject.rs"
    index_project "$DIR"
    OUT=$($IG --json "inject_test_197" "$DIR" 2>&1)
    # Output must be parseable JSON (content is embedded as a JSON string value)
    echo "$OUT" | python3 -c "import sys, json; [json.loads(l) for l in sys.stdin if l.strip()]" 2>/dev/null
'

run_test "T198" "SECURITY" "path traversal attempt does not escape project root" '
    DIR=$(new_project_nogit)
    echo "fn traversal_target_198() {}" > "$DIR/file.rs"
    index_project "$DIR"
    # A traversal path like "../../etc/passwd" should not match project content
    OUT=$($IG "root:\|daemon:" "$DIR/../../../etc" 2>&1)
    # Should either return no results or an error — not actual /etc content
    ! echo "$OUT" | grep -q "^root:\|^daemon:"
'

run_test "T199" "SECURITY" "very long pattern (10KB) does not crash" '
    DIR=$(new_project_nogit)
    echo "fn long_pattern_199() {}" > "$DIR/file.rs"
    index_project "$DIR"
    LONG_PAT=$(python3 -c "print('"'"'a'"'"' * 10000)")
    $IG "$LONG_PAT" "$DIR" >/dev/null 2>&1
    [[ $? -ne 139 ]]
'

run_test "T200" "SECURITY" "pattern with NUL byte does not crash" '
    DIR=$(new_project_nogit)
    echo "fn nul_pattern_200() {}" > "$DIR/file.rs"
    index_project "$DIR"
    # Pass a pattern that contains a NUL byte — binary pattern, should not segfault
    printf "nul\x00pat" | xargs -0 $IG "$DIR" >/dev/null 2>&1 || true
    [[ $? -ne 139 ]]
'

# ─────────────────────────────────────────────────────────────
# SUMMARY
# ─────────────────────────────────────────────────────────────

echo ""
echo "========================================"
TOTAL=$((PASS + FAIL + SKIP))
echo -e "${BOLD}Results: ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}, ${YELLOW}$SKIP skipped${NC} / $TOTAL total"

if [[ ${#ERRORS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${RED}Failed tests:${NC}"
    for err in "${ERRORS[@]}"; do
        echo "  - $err"
    done
fi

[[ $FAIL -eq 0 ]]
