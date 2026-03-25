#!/usr/bin/env bash
# ============================================================================
# instant-grep (ig) Benchmark Suite
# Compare ig vs ripgrep (rg) vs grep on any git repository
#
# Usage:
#   ./benchmarks/benchmark.sh                         # benchmark current repo
#   ./benchmarks/benchmark.sh /path/to/repo1 /path/to/repo2
#   ./benchmarks/benchmark.sh --quick /path/to/repo   # 3 runs instead of 10
#   ./benchmarks/benchmark.sh --no-grep /path/to/repo  # skip grep (very slow)
#
# Repos: pass any number of git repo paths as arguments.
#        If none given, benchmarks the current directory.
# ============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Parse flags
# ---------------------------------------------------------------------------

RUNS=10
SKIP_GREP=false
REPO_ARGS=()

for arg in "$@"; do
  case "$arg" in
    --quick)  RUNS=3 ;;
    --no-grep) SKIP_GREP=true ;;
    --help|-h)
      echo "Usage: $0 [--quick] [--no-grep] [path/to/repo ...]"
      echo ""
      echo "  --quick     3 runs per test instead of 10"
      echo "  --no-grep   skip grep (can be very slow on large repos)"
      echo "  path...     git repos to benchmark (default: current dir)"
      exit 0
      ;;
    *) REPO_ARGS+=("$arg") ;;
  esac
done

# Default to current directory if no repos given
if [[ ${#REPO_ARGS[@]} -eq 0 ]]; then
  REPO_ARGS=(".")
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_FILE="$SCRIPT_DIR/RESULTS.md"

# Test patterns: label|flags|pattern
PATTERNS=(
  'literal-common||function'
  'regex||class\s+\w+'
  'case-insensitive|-i|todo'
  'no-match||zzzzxqwk_nomatch'
  'literal-rare||deprecated'
  'import||import'
)

# ---------------------------------------------------------------------------
# Tool detection
# ---------------------------------------------------------------------------

declare -A TOOLS
IG_VERSION="" ; RG_VERSION="" ; GREP_VERSION=""

if command -v ig &>/dev/null; then
  TOOLS[ig]="$(command -v ig)"
  IG_VERSION="$(ig --version 2>&1 || echo 'unknown')"
else
  echo "ERROR: ig not found. Install it first: https://github.com/MakFly/instant-grep"
  exit 1
fi

if command -v rg &>/dev/null; then
  TOOLS[rg]="$(command -v rg)"
  RG_VERSION="$(rg --version 2>&1 | head -1 || echo 'unknown')"
else
  echo "WARNING: rg not found, skipping ripgrep"
fi

GREP_BIN=""
if [[ "$SKIP_GREP" == "false" ]]; then
  if [[ -x /usr/bin/grep ]]; then
    GREP_BIN="/usr/bin/grep"
  elif [[ -x /opt/homebrew/bin/ggrep ]]; then
    GREP_BIN="/opt/homebrew/bin/ggrep"
  fi
  if [[ -n "$GREP_BIN" ]]; then
    TOOLS[grep]="$GREP_BIN"
    GREP_VERSION="$($GREP_BIN --version 2>&1 | head -1 || echo 'unknown')"
  fi
fi

echo "Tools: ${!TOOLS[*]}"
[[ $RUNS -lt 10 ]] && echo "Quick mode: $RUNS runs"
echo ""

# ---------------------------------------------------------------------------
# Resolve repos — get absolute paths and file counts
# ---------------------------------------------------------------------------

REPOS=()
for repo_arg in "${REPO_ARGS[@]}"; do
  repo_path="$(cd "$repo_arg" 2>/dev/null && pwd || echo "")"
  if [[ -z "$repo_path" || ! -d "$repo_path" ]]; then
    echo "WARNING: $repo_arg not found, skipping"
    continue
  fi
  repo_name="$(basename "$repo_path")"
  file_count="$(git -C "$repo_path" ls-files 2>/dev/null | wc -l | tr -d ' ')"
  REPOS+=("$repo_name|$repo_path|$file_count")
done

if [[ ${#REPOS[@]} -eq 0 ]]; then
  echo "ERROR: No valid repos to benchmark."
  exit 1
fi

# ---------------------------------------------------------------------------
# Ensure ig indexes exist
# ---------------------------------------------------------------------------

echo "Building ig indexes..."
for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r name path _ <<< "$repo_entry"
  echo "  $name ($path)"
  (cd "$path" && ig index) 2>&1 | tail -1
done
echo ""

# ---------------------------------------------------------------------------
# Machine info
# ---------------------------------------------------------------------------

HOSTNAME_STR="$(hostname 2>/dev/null || echo 'unknown')"
OS_STR="$(uname -srm 2>/dev/null || echo 'unknown')"
CPU_STR="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | xargs || echo 'unknown')"
CPU_CORES="$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo '?')"
RAM_GB="$(python3 -c "
import os
try:
    import subprocess
    b = int(subprocess.check_output(['sysctl', '-n', 'hw.memsize']).strip())
except:
    b = os.sysconf('SC_PAGE_SIZE') * os.sysconf('SC_PHYS_PAGES')
print(f'{b / (1024**3):.1f}')
" 2>/dev/null || echo '?')"

echo "============================================"
echo "  ig Benchmark Suite"
echo "============================================"
echo "Host:    $HOSTNAME_STR"
echo "OS:      $OS_STR"
echo "CPU:     $CPU_STR ($CPU_CORES cores)"
echo "RAM:     ${RAM_GB} GB"
echo "Runs:    $RUNS per test"
echo ""
[[ -n "$IG_VERSION" ]]   && echo "ig:      $IG_VERSION"
[[ -n "$RG_VERSION" ]]   && echo "rg:      $RG_VERSION"
[[ -n "$GREP_VERSION" ]] && echo "grep:    $GREP_VERSION"
echo "============================================"
echo ""

# ---------------------------------------------------------------------------
# Python timing helper
# ---------------------------------------------------------------------------

TIMER_PY='
import sys, time, subprocess, json

cmd = sys.argv[1]
runs = int(sys.argv[2])
times = []
match_count = 0

for i in range(runs):
    start = time.time()
    result = subprocess.run(cmd, shell=True, capture_output=True)
    elapsed = time.time() - start
    times.append(elapsed)
    if i == runs - 1:
        match_count = len(result.stdout.decode("utf-8", errors="replace").splitlines())

times.sort()
n = len(times)
median = times[n // 2] if n % 2 == 1 else (times[n // 2 - 1] + times[n // 2]) / 2

print(json.dumps({
    "median_ms": round(median * 1000, 2),
    "min_ms": round(min(times) * 1000, 2),
    "max_ms": round(max(times) * 1000, 2),
    "matches": match_count,
    "runs": runs
}))
'

# ---------------------------------------------------------------------------
# Temp results file
# ---------------------------------------------------------------------------

TMPRESULTS="/tmp/ig_bench_$$.jsonl"
rm -f "$TMPRESULTS"
trap 'rm -f "$TMPRESULTS"' EXIT

# ---------------------------------------------------------------------------
# Build search command for each tool
# ---------------------------------------------------------------------------

build_cmd() {
  local tool="$1" flags="$2" pattern="$3" path="$4"
  case "$tool" in
    ig)
      if [[ -n "$flags" ]]; then
        echo "cd '$path' && ig $flags '$pattern' 2>/dev/null"
      else
        echo "cd '$path' && ig '$pattern' 2>/dev/null"
      fi
      ;;
    rg)
      if [[ -n "$flags" ]]; then
        echo "rg --no-heading $flags '$pattern' '$path' 2>/dev/null"
      else
        echo "rg --no-heading '$pattern' '$path' 2>/dev/null"
      fi
      ;;
    grep)
      if [[ -n "$flags" ]]; then
        echo "$GREP_BIN -rn $flags '$pattern' '$path' 2>/dev/null"
      else
        echo "$GREP_BIN -rn '$pattern' '$path' 2>/dev/null"
      fi
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Run benchmarks
# ---------------------------------------------------------------------------

echo "Running benchmarks..."
echo ""

for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r repo_name repo_path file_count <<< "$repo_entry"
  echo "--- $repo_name ($file_count files) ---"
  echo ""

  for pattern_entry in "${PATTERNS[@]}"; do
    IFS='|' read -r label flags pattern <<< "$pattern_entry"
    printf "  %-24s" "$label:"

    for tool in ig rg grep; do
      [[ -z "${TOOLS[$tool]:-}" ]] && continue

      cmd="$(build_cmd "$tool" "$flags" "$pattern" "$repo_path")"
      result="$(python3 -c "$TIMER_PY" "$cmd" "$RUNS" 2>/dev/null || echo '{"median_ms":0,"min_ms":0,"max_ms":0,"matches":0,"runs":0}')"

      median=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['median_ms'])")
      min_t=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['min_ms'])")
      max_t=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['max_ms'])")
      matches=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['matches'])")

      printf "  %s: %7sms" "$tool" "$median"

      echo "{\"repo\":\"$repo_name\",\"files\":$file_count,\"pattern\":\"$label\",\"tool\":\"$tool\",\"median\":$median,\"min\":$min_t,\"max\":$max_t,\"matches\":$matches}" >> "$TMPRESULTS"
    done
    echo ""
  done
  echo ""
done

# ---------------------------------------------------------------------------
# Generate RESULTS.md
# ---------------------------------------------------------------------------

echo "Generating $RESULTS_FILE ..."

cat > "$RESULTS_FILE" <<HEADER
# Benchmark Results

**Date:** $(date '+%Y-%m-%d %H:%M:%S')
**Host:** $HOSTNAME_STR
**OS:** $OS_STR
**CPU:** $CPU_STR ($CPU_CORES cores)
**RAM:** ${RAM_GB} GB
**Runs:** $RUNS per test (median reported)

## Tool Versions

HEADER

[[ -n "$IG_VERSION" ]]   && echo "- **ig:** $IG_VERSION" >> "$RESULTS_FILE"
[[ -n "$RG_VERSION" ]]   && echo "- **rg:** $RG_VERSION" >> "$RESULTS_FILE"
[[ -n "$GREP_VERSION" ]] && echo "- **grep:** $GREP_VERSION" >> "$RESULTS_FILE"

for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r repo_name _ file_count <<< "$repo_entry"

  cat >> "$RESULTS_FILE" <<REPOHEADER

## $repo_name (~$file_count files)

| Pattern | Tool | Median (ms) | Min (ms) | Max (ms) | Matches |
|---------|------|------------:|---------:|---------:|--------:|
REPOHEADER

  python3 -c "
import json
results = []
for line in open('$TMPRESULTS'):
    line = line.strip()
    if not line: continue
    r = json.loads(line)
    if r['repo'] == '$repo_name':
        results.append(r)
results.sort(key=lambda r: (r['pattern'], r['median']))
for r in results:
    print(f\"| {r['pattern']} | {r['tool']} | {r['median']} | {r['min']} | {r['max']} | {r['matches']} |\")
" >> "$RESULTS_FILE"
done

# Summary table
cat >> "$RESULTS_FILE" <<'SUMMARY_HEADER'

## Summary: Fastest Tool per Test

| Repo | Files | Pattern | Winner | Median (ms) | vs 2nd |
|------|------:|---------|--------|------------:|-------:|
SUMMARY_HEADER

python3 -c "
import json
from collections import defaultdict
results = defaultdict(list)
for line in open('$TMPRESULTS'):
    line = line.strip()
    if not line: continue
    r = json.loads(line)
    results[(r['repo'], r['files'], r['pattern'])].append(r)
for key in sorted(results.keys()):
    entries = sorted(results[key], key=lambda x: x['median'])
    w = entries[0]
    speedup = f\"{entries[1]['median']/w['median']:.1f}x\" if len(entries) > 1 and w['median'] > 0 else 'N/A'
    print(f\"| {w['repo']} | {w['files']} | {w['pattern']} | **{w['tool']}** | {w['median']} | {speedup} |\")
" >> "$RESULTS_FILE"

echo "" >> "$RESULTS_FILE"
echo "_Generated by \`benchmarks/benchmark.sh\`_" >> "$RESULTS_FILE"

echo ""
echo "Results written to $RESULTS_FILE"
echo "Done."
