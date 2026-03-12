#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# ngrep vs ripgrep — benchmark suite
#
# Usage:
#   ./bench-vs-rg.sh [target_dir]
#
# Defaults to current directory. Requires: ngrep, rg, perl (with Time::HiRes)
# ═══════════════════════════════════════════════════════════════════

set -euo pipefail

TARGET="${1:-.}"
RUNS=5

# ── Colors ────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
RESET='\033[0m'

# ── Preflight ─────────────────────────────────────────────────────
for cmd in ngrep rg perl; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "Error: $cmd not found in PATH" >&2
    exit 1
  fi
done

perl -MTime::HiRes -e '1' 2>/dev/null || {
  echo "Error: perl Time::HiRes module not available" >&2
  exit 1
}

# ── Repo stats ────────────────────────────────────────────────────
file_count=$(rg --files "$TARGET" 2>/dev/null | wc -l | tr -d ' ')
echo -e "${BOLD}ngrep vs ripgrep — benchmark suite${RESET}"
echo -e "${DIM}Target: ${TARGET}${RESET}"
echo -e "${DIM}Files:  ${file_count}${RESET}"
echo -e "${DIM}Runs:   best of ${RUNS}${RESET}"
echo ""

# ── Timing helper ─────────────────────────────────────────────────
# Returns wall-clock microseconds for a single invocation
time_us() {
  local cmd="$1"
  shift
  perl -MTime::HiRes=time -e '
    my $s = time();
    open(STDOUT_SAVE, ">&STDOUT");
    open(STDOUT, ">/dev/null");
    system(@ARGV);
    open(STDOUT, ">&STDOUT_SAVE");
    printf "%.0f\n", (time() - $s) * 1000000;
  ' -- "$cmd" "$@" 2>/dev/null
}

# Best-of-N microseconds
best_of() {
  local n="$1" cmd="$2"
  shift 2
  local best=999999999
  for ((i = 0; i < n; i++)); do
    local us
    us=$(time_us "$cmd" "$@")
    if [[ "$us" -lt "$best" ]]; then
      best=$us
    fi
  done
  echo "$best"
}

# Format microseconds as milliseconds string
fmt_ms() {
  echo "scale=2; $1 / 1000" | bc
}

# ── Benchmark runners ─────────────────────────────────────────────

# Quiet mode: search only, no output (-q)
bench_quiet() {
  local label="$1" pattern="$2"

  local ng_us rg_us
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -q "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -q "$TARGET")

  local ng_ms rg_ms
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  local winner ratio
  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$label" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
}

# With output: line numbers, piped to /dev/null
bench_output() {
  local label="$1" pattern="$2"

  local ng_us rg_us
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -n "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -n "$TARGET")

  local ng_ms rg_ms
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  local matches
  matches=$(ngrep "$pattern" -c "$TARGET" 2>/dev/null | awk -F: '{s+=$NF}END{print s+0}')

  local winner ratio
  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-30s | %5s | %8s | %8s | " "$label" "$matches" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
}

# Files-only mode: list matching files (-l)
bench_files() {
  local label="$1" pattern="$2"

  local ng_us rg_us
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -l "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -l "$TARGET")

  local ng_ms rg_ms
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  local file_count
  file_count=$(ngrep "$pattern" -l "$TARGET" 2>/dev/null | wc -l | tr -d ' ')

  local winner ratio
  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-30s | %5s | %8s | %8s | " "$label" "$file_count" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
}

# Count mode: match counts per file (-c)
bench_count() {
  local label="$1" pattern="$2"

  local ng_us rg_us
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -c "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -c "$TARGET")

  local ng_ms rg_ms
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  local winner ratio
  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$label" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
}

# Context mode: lines before/after (-C)
bench_context() {
  local label="$1" pattern="$2" ctx="$3"

  local ng_us rg_us
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -C "$ctx" "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -C "$ctx" "$TARGET")

  local ng_ms rg_ms
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  local winner ratio
  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$label" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
}

# ══════════════════════════════════════════════════════════════════
# TEST 1: Quiet mode (search decision speed)
# ══════════════════════════════════════════════════════════════════
echo -e "${BOLD}${CYAN}TEST 1: QUIET MODE${RESET} ${DIM}(search only, -q, no output)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

bench_quiet "getServiceClient (literal)"        "getServiceClient"
bench_quiet "import {} from (regex)"            'import\s+\{.*\}\s+from'
bench_quiet "process.env.* (regex)"             'process\.env\.\w+'
bench_quiet "async function (regex)"            'async\s+function\s+\w+'
bench_quiet "supabase.from( (escaped)"          'supabase\.from\('
bench_quiet "TODO|FIXME|HACK (alternation)"     'TODO|FIXME|HACK'
bench_quiet "export function (regex)"           'export\s+(default\s+)?function'
bench_quiet "createClient (literal)"            'createClient'
bench_quiet ".update(.*as never) (regex)"       '\.update\(.*as never\)'
bench_quiet "console.log|err|warn (altern)"     'console\.(log|error|warn)'

# ══════════════════════════════════════════════════════════════════
# TEST 2: Output mode (search + format + print)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 2: OUTPUT MODE${RESET} ${DIM}(line numbers, -n, to /dev/null)${RESET}"
echo ""
printf "| %-30s | %5s | %8s | %8s | %s\n" "Pattern" "Hits" "ngrep" "rg" "Winner"
printf "|%-32s|%7s|%10s|%10s|%s\n" "--------------------------------" "-------" "----------" "----------" "----------"

bench_output "getServiceClient"            "getServiceClient"
bench_output "import {} from"              'import\s+\{.*\}\s+from'
bench_output "console.log/err/warn"        'console\.(log|error|warn)'
bench_output "all function defs"           'function\s+\w+'
bench_output "supabase.from("              'supabase\.from\('

# ══════════════════════════════════════════════════════════════════
# TEST 3: Files-only mode (-l)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 3: FILES-ONLY MODE${RESET} ${DIM}(-l, list matching files)${RESET}"
echo ""
printf "| %-30s | %5s | %8s | %8s | %s\n" "Pattern" "Files" "ngrep" "rg" "Winner"
printf "|%-32s|%7s|%10s|%10s|%s\n" "--------------------------------" "-------" "----------" "----------" "----------"

bench_files "getServiceClient"             "getServiceClient"
bench_files "process.env"                  'process\.env\.'
bench_files "import from"                  'import\s+.*from'
bench_files "TODO|FIXME"                   'TODO|FIXME'

# ══════════════════════════════════════════════════════════════════
# TEST 4: Count mode (-c)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 4: COUNT MODE${RESET} ${DIM}(-c, match counts per file)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

bench_count "getServiceClient"              "getServiceClient"
bench_count "import {} from"                'import\s+\{.*\}\s+from'
bench_count "console.log/err/warn"          'console\.(log|error|warn)'
bench_count ". (every line)"                '.'

# ══════════════════════════════════════════════════════════════════
# TEST 5: Context mode (-C)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 5: CONTEXT MODE${RESET} ${DIM}(-C N, lines before/after)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

bench_context "getServiceClient -C2"        "getServiceClient" 2
bench_context "async function -C3"          'async\s+function\s+\w+' 3
bench_context "TODO|FIXME -C5"              'TODO|FIXME' 5
bench_context "console.log/err/warn -C2"    'console\.(log|error|warn)' 2

# ══════════════════════════════════════════════════════════════════
# TEST 6: Whole-repo scan (match every line)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 6: WHOLE-REPO SCAN${RESET} ${DIM}(match every line in .ts files, -c)${RESET}"
echo ""

ng_us=$(best_of "$RUNS" ngrep "." -t ts -c "$TARGET")
rg_us=$(best_of "$RUNS" rg "." -t ts -c "$TARGET")
ng_ms=$(fmt_ms "$ng_us")
rg_ms=$(fmt_ms "$rg_us")

if [[ "$ng_us" -lt "$rg_us" ]]; then
  ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
  echo -e "ngrep: ${GREEN}${ng_ms}ms${RESET} | rg: ${rg_ms}ms | ${GREEN}ngrep ${ratio}x faster${RESET}"
else
  ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
  echo -e "ngrep: ${ng_ms}ms | rg: ${RED}${rg_ms}ms${RESET} | ${RED}rg ${ratio}x faster${RESET}"
fi

# ══════════════════════════════════════════════════════════════════
# TEST 7: File type filter (-t)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 7: FILE TYPE FILTER${RESET} ${DIM}(-t ts, search only .ts files)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

for pattern in "getServiceClient" 'import\s+\{.*\}\s+from' 'async\s+function' 'console\.log'; do
  label=$(echo "$pattern" | sed 's/\\s+/ /g; s/\\[{}()|.]//g' | cut -c1-34)

  ng_us=$(best_of "$RUNS" ngrep "$pattern" -t ts -q "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -t ts -q "$TARGET")
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$label" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
done

# ══════════════════════════════════════════════════════════════════
# TEST 8: Glob filter (-g)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 8: GLOB FILTER${RESET} ${DIM}(-g '*.ts', search specific file patterns)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

for pattern in "getServiceClient" 'import\s+\{.*\}\s+from' 'async\s+function'; do
  label="$(echo "$pattern" | sed 's/\\s+/ /g; s/\\[{}()|.]//g' | cut -c1-30) -g*.ts"

  ng_us=$(best_of "$RUNS" ngrep "$pattern" -g '*.ts' -q "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -g '*.ts' -q "$TARGET")
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$label" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
done

# ══════════════════════════════════════════════════════════════════
# TEST 9: Case-insensitive (-i)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 9: CASE-INSENSITIVE${RESET} ${DIM}(-i)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

for pattern in "error" "function" "import"; do
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -i -q "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -i -q "$TARGET")
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "$pattern -i" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
done

# ══════════════════════════════════════════════════════════════════
# TEST 10: Inverted match (-v)
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}TEST 10: INVERTED MATCH${RESET} ${DIM}(-v, lines NOT matching)${RESET}"
echo ""
printf "| %-34s | %8s | %8s | %s\n" "Pattern" "ngrep" "rg" "Winner"
printf "|%-36s|%10s|%10s|%s\n" "------------------------------------" "----------" "----------" "----------"

for pattern in "import" "const" "//"; do
  ng_us=$(best_of "$RUNS" ngrep "$pattern" -v -c "$TARGET")
  rg_us=$(best_of "$RUNS" rg "$pattern" -v -c "$TARGET")
  ng_ms=$(fmt_ms "$ng_us")
  rg_ms=$(fmt_ms "$rg_us")

  if [[ "$ng_us" -lt "$rg_us" ]]; then
    ratio=$(echo "scale=1; $rg_us / $ng_us" | bc)
    winner="${GREEN}ngrep ${ratio}x${RESET}"
  elif [[ "$rg_us" -lt "$ng_us" ]]; then
    ratio=$(echo "scale=1; $ng_us / $rg_us" | bc)
    winner="${RED}rg ${ratio}x${RESET}"
  else
    winner="tie"
  fi

  printf "| %-34s | %8s | %8s | " "NOT $pattern (-v -c)" "${ng_ms}ms" "${rg_ms}ms"
  echo -e "$winner"
done

# ══════════════════════════════════════════════════════════════════
# SUMMARY
# ══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════════${RESET}"
echo -e "${BOLD}Done.${RESET} ${DIM}Best of ${RUNS} runs per test, microsecond precision.${RESET}"
echo -e "${DIM}Green = ngrep faster, Red = rg faster${RESET}"
