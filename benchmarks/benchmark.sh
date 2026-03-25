#!/usr/bin/env bash
# ============================================================================
# instant-grep (ig) Benchmark Suite
# Compare ig v1.1.0 vs ripgrep (rg) vs grep
#
# Usage:
#   ./benchmarks/benchmark.sh           # 10 runs per test (default)
#   ./benchmarks/benchmark.sh --quick   # 3 runs per test
# ============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

RUNS=10
if [[ "${1:-}" == "--quick" ]]; then
  RUNS=3
  echo "Quick mode: $RUNS runs per test"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_FILE="$SCRIPT_DIR/RESULTS.md"

# Repos to benchmark (name, path, expected file count)
REPOS=(
  "trading-app|/Users/kev/Documents/lab/pro/tilvest/trading-app|11350"
  "distribution-app-v2|/Users/kev/Documents/lab/pro/tilvest/distribution-app-v2|3101"
  "headless-kit|/Users/kev/Documents/lab/sandbox/headless-kit-next-php-hono|2541"
)

# Test patterns: label|flags|pattern
# flags: "" = default, "-i" = case-insensitive
PATTERNS=(
  'literal-common||function'
  'regex||class\s+\w+'
  'case-insensitive-rare|-i|todo'
  'no-match||zzzzxqwk_nomatch'
  'literal-rare||deprecated'
  'import||import'
)

# ---------------------------------------------------------------------------
# Tool detection
# ---------------------------------------------------------------------------

declare -A TOOLS

# ig
if command -v ig &>/dev/null; then
  TOOLS[ig]="$(command -v ig)"
  IG_VERSION="$(ig --version 2>&1 || echo 'unknown')"
else
  echo "WARNING: ig not found, skipping"
fi

# ripgrep
if command -v rg &>/dev/null; then
  TOOLS[rg]="$(command -v rg)"
  RG_VERSION="$(rg --version 2>&1 | head -1 || echo 'unknown')"
else
  echo "WARNING: rg not found, skipping"
fi

# grep — find the real binary, bypass aliases
GREP_BIN=""
if [[ -x /usr/bin/grep ]]; then
  GREP_BIN="/usr/bin/grep"
elif [[ -x /opt/homebrew/bin/ggrep ]]; then
  GREP_BIN="/opt/homebrew/bin/ggrep"
fi
if [[ -n "$GREP_BIN" ]]; then
  TOOLS[grep]="$GREP_BIN"
  GREP_VERSION="$($GREP_BIN --version 2>&1 | head -1 || echo 'unknown')"
else
  echo "WARNING: grep not found, skipping"
fi

if [[ ${#TOOLS[@]} -eq 0 ]]; then
  echo "ERROR: No search tools found. Aborting."
  exit 1
fi

echo "Tools detected: ${!TOOLS[*]}"
echo ""

# ---------------------------------------------------------------------------
# Python timing helper (more reliable than bash time)
# ---------------------------------------------------------------------------

TIMER_PY=$(cat <<'PYEOF'
import sys, time, subprocess, json, statistics

cmd = sys.argv[1]
runs = int(sys.argv[2])

times = []
match_count = 0

for i in range(runs):
    start = time.time()
    result = subprocess.run(cmd, shell=True, capture_output=True)
    elapsed = time.time() - start
    times.append(elapsed)
    # Count matches from last run
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
PYEOF
)

# ---------------------------------------------------------------------------
# Ensure ig indexes are built for all repos
# ---------------------------------------------------------------------------

echo "Building/verifying ig indexes..."
for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r name path expected <<< "$repo_entry"
  if [[ ! -d "$path" ]]; then
    echo "  SKIP: $path does not exist"
    continue
  fi
  if [[ -n "${TOOLS[ig]:-}" ]]; then
    echo "  Indexing $name ($path)..."
    (cd "$path" && ig index) 2>&1 | tail -1
  fi
done
echo ""

# ---------------------------------------------------------------------------
# Machine info
# ---------------------------------------------------------------------------

HOSTNAME_STR="$(hostname 2>/dev/null || echo 'unknown')"
OS_STR="$(uname -srm 2>/dev/null || echo 'unknown')"
CPU_STR="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo 'unknown')"
CPU_CORES="$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo '?')"
RAM_BYTES="$(sysctl -n hw.memsize 2>/dev/null || echo '0')"
RAM_GB=$(python3 -c "print(round($RAM_BYTES / (1024**3), 1))")

echo "============================================"
echo "  instant-grep Benchmark Suite"
echo "============================================"
echo "Host:    $HOSTNAME_STR"
echo "OS:      $OS_STR"
echo "CPU:     $CPU_STR ($CPU_CORES cores)"
echo "RAM:     ${RAM_GB} GB"
echo "Runs:    $RUNS per test"
echo ""
[[ -n "${TOOLS[ig]:-}" ]]    && echo "ig:      $IG_VERSION"
[[ -n "${TOOLS[rg]:-}" ]]    && echo "rg:      $RG_VERSION"
[[ -n "${TOOLS[grep]:-}" ]]  && echo "grep:    $GREP_VERSION"
echo "============================================"
echo ""

# ---------------------------------------------------------------------------
# Results storage (for markdown output)
# ---------------------------------------------------------------------------

# Temp file for collecting all results, sorted later
TMPRESULTS="/tmp/bench_results_$$.jsonl"
rm -f "$TMPRESULTS"
trap "rm -f $TMPRESULTS" EXIT

# ---------------------------------------------------------------------------
# Build command for each tool
# ---------------------------------------------------------------------------

build_cmd() {
  local tool="$1"
  local flags="$2"
  local pattern="$3"
  local path="$4"

  case "$tool" in
    ig)
      # ig: flags go before pattern
      if [[ -n "$flags" ]]; then
        echo "cd '$path' && ig $flags '$pattern' 2>/dev/null"
      else
        echo "cd '$path' && ig '$pattern' 2>/dev/null"
      fi
      ;;
    rg)
      # rg: similar flags, add --no-heading for consistent output
      if [[ -n "$flags" ]]; then
        echo "rg --no-heading $flags '$pattern' '$path' 2>/dev/null"
      else
        echo "rg --no-heading '$pattern' '$path' 2>/dev/null"
      fi
      ;;
    grep)
      # grep: -rn plus any extra flags
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

total_tests=0
for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r repo_name repo_path _ <<< "$repo_entry"

  if [[ ! -d "$repo_path" ]]; then
    echo "SKIP: $repo_path not found"
    continue
  fi

  echo "--- $repo_name ($repo_path) ---"
  echo ""

  for pattern_entry in "${PATTERNS[@]}"; do
    IFS='|' read -r label flags pattern <<< "$pattern_entry"

    printf "  %-28s" "$label:"

    for tool in ig rg grep; do
      [[ -z "${TOOLS[$tool]:-}" ]] && continue

      cmd="$(build_cmd "$tool" "$flags" "$pattern" "$repo_path")"
      result="$(python3 -c "$TIMER_PY" "$cmd" "$RUNS" 2>/dev/null)"

      median=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['median_ms'])")
      min_t=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['min_ms'])")
      max_t=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['max_ms'])")
      matches=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['matches'])")

      printf "  %s: %7s ms" "$tool" "$median"

      # Save for markdown
      echo "{\"repo\":\"$repo_name\",\"pattern\":\"$label\",\"tool\":\"$tool\",\"median\":$median,\"min\":$min_t,\"max\":$max_t,\"matches\":$matches}" >> "$TMPRESULTS"

      total_tests=$((total_tests + 1))
    done

    echo ""
  done

  echo ""
done

echo "Completed $total_tests test runs."
echo ""

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

[[ -n "${TOOLS[ig]:-}" ]]   && echo "- **ig:** $IG_VERSION" >> "$RESULTS_FILE"
[[ -n "${TOOLS[rg]:-}" ]]   && echo "- **rg:** $RG_VERSION" >> "$RESULTS_FILE"
[[ -n "${TOOLS[grep]:-}" ]] && echo "- **grep:** $GREP_VERSION" >> "$RESULTS_FILE"

# Generate one table per repo
for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r repo_name repo_path expected_files <<< "$repo_entry"

  cat >> "$RESULTS_FILE" <<REPOHEADER

## $repo_name (~$expected_files files)

| Pattern | Tool | Median (ms) | Min (ms) | Max (ms) | Matches |
|---------|------|------------:|----------|---------:|--------:|
REPOHEADER

  # Extract results for this repo, sort by pattern then median time
  python3 -c "
import json, sys

results = []
for line in open('$TMPRESULTS'):
    line = line.strip()
    if not line:
        continue
    r = json.loads(line)
    if r['repo'] == '$repo_name':
        results.append(r)

# Sort by pattern name, then by median time (fastest first)
results.sort(key=lambda r: (r['pattern'], r['median']))

for r in results:
    print(f\"| {r['pattern']} | {r['tool']} | {r['median']} | {r['min']} | {r['max']} | {r['matches']} |\")
" >> "$RESULTS_FILE"

done

# Summary: winner per pattern across all repos
cat >> "$RESULTS_FILE" <<'SUMMARY_HEADER'

## Summary: Fastest Tool per Test

| Repo | Pattern | Winner | Median (ms) | Speedup vs 2nd |
|------|---------|--------|------------:|---------------:|
SUMMARY_HEADER

python3 -c "
import json
from collections import defaultdict

results = defaultdict(list)
for line in open('$TMPRESULTS'):
    line = line.strip()
    if not line:
        continue
    r = json.loads(line)
    key = (r['repo'], r['pattern'])
    results[key].append(r)

for key in sorted(results.keys()):
    entries = sorted(results[key], key=lambda x: x['median'])
    winner = entries[0]
    if len(entries) > 1 and entries[0]['median'] > 0:
        speedup = round(entries[1]['median'] / entries[0]['median'], 1)
        speedup_str = f'{speedup}x'
    else:
        speedup_str = 'N/A'
    print(f\"| {winner['repo']} | {winner['pattern']} | **{winner['tool']}** | {winner['median']} | {speedup_str} |\")
" >> "$RESULTS_FILE"

echo "" >> "$RESULTS_FILE"
echo "_Generated by benchmarks/benchmark.sh_" >> "$RESULTS_FILE"

echo ""
echo "Results written to $RESULTS_FILE"
echo "Done."
