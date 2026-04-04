#!/usr/bin/env bash
# check-plug-and-play.sh — Automated plug-and-play gate for all modules.
#
# Runs 12 deterministic checks (G1–G12) against every module under modules/
# and produces a markdown pass/fail matrix.
#
# Usage:
#   ./scripts/check-plug-and-play.sh          # full table
#   ./scripts/check-plug-and-play.sh --json   # machine-readable JSON
#
# Exit code: 0 if all modules pass all gates, 1 otherwise.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODULES_DIR="$PROJECT_ROOT/modules"
CLIENTS_DIR="$PROJECT_ROOT/clients"
ALLOWLIST="$PROJECT_ROOT/.file-size-allowlist"
G4_ALLOWLIST="$PROJECT_ROOT/.g4-allowlist"
G7_ALLOWLIST="$PROJECT_ROOT/.g7-subject-allowlist"
G10_ALLOWLIST="$PROJECT_ROOT/.g10-allowlist"

OUTPUT_JSON=false
[[ "${1:-}" == "--json" ]] && OUTPUT_JSON=true

# ── Module list (sorted) ────────────────────────────────────────
MODULES=()
for d in "$MODULES_DIR"/*/; do
  [ -d "$d" ] && MODULES+=("$(basename "$d")")
done
IFS=$'\n' MODULES=($(sort <<<"${MODULES[*]}")); unset IFS

# ── Helpers ──────────────────────────────────────────────────────

# Count lines in a file (excluding blanks/comments)
loc() {
  grep -cve '^\s*$' -e '^\s*//' "$1" 2>/dev/null || echo 0
}

# Check if a path is in the allowlist
in_allowlist() {
  local relpath="$1"
  [ -f "$ALLOWLIST" ] && grep -qF "$relpath" "$ALLOWLIST" 2>/dev/null
}

# Check if a module is in the G4 allowlist (dated entries: "module YYYY-MM-DD reason")
in_g4_allowlist() {
  local mod="$1"
  [ -f "$G4_ALLOWLIST" ] && grep -q "^$mod " "$G4_ALLOWLIST" 2>/dev/null
}

# Check if a module is in the G10 vendor HTTP allowlist (dated entries: "module YYYY-MM-DD reason")
in_g10_allowlist() {
  local mod="$1"
  [ -f "$G10_ALLOWLIST" ] && grep -q "^$mod " "$G10_ALLOWLIST" 2>/dev/null
}

# ── Gate functions ───────────────────────────────────────────────
# Each takes a module name and prints PASS or FAIL.

# G1: PaginatedResponse used on list/search endpoints
gate_g1() {
  local mod="$1"
  local http_dir="$MODULES_DIR/$mod/src/http"
  if [ ! -d "$http_dir" ]; then
    echo "SKIP"
    return
  fi
  # Check if any handler file references PaginatedResponse
  if grep -rql "PaginatedResponse" "$http_dir" 2>/dev/null; then
    echo "PASS"
  else
    # If there are list/search endpoints but no PaginatedResponse, fail
    if grep -rqE "get\(.*list_|get\(.*search_" "$http_dir" 2>/dev/null || \
       grep -rqE "fn list_|fn search_" "$http_dir" 2>/dev/null; then
      echo "FAIL"
    else
      # No list endpoints — PaginatedResponse not needed
      echo "N/A"
    fi
  fi
}

# G2: ApiError used on all error paths
gate_g2() {
  local mod="$1"
  local http_dir="$MODULES_DIR/$mod/src/http"
  if [ ! -d "$http_dir" ]; then
    echo "SKIP"
    return
  fi
  if grep -rql "ApiError" "$http_dir" 2>/dev/null; then
    echo "PASS"
  else
    echo "FAIL"
  fi
}

# G3: utoipa annotations exist
gate_g3() {
  local mod="$1"
  local src_dir="$MODULES_DIR/$mod/src"
  local count
  count=$(grep -rl "utoipa::path\|#\[utoipa" "$src_dir" 2>/dev/null | wc -l | tr -d ' ')
  if [ "$count" -gt 0 ]; then
    echo "PASS"
  else
    echo "FAIL"
  fi
}

# G4: utoipa coverage — annotations vs handler functions (>=80%)
#
# Only counts pub async fn in files that contain at least one #[utoipa::path]
# annotation. This excludes repo helpers, session logic, tenant extractors,
# and other non-handler code that lives under http/ but is never mounted
# on the Axum router. If a file has zero utoipa annotations it is assumed
# to contain no endpoints and is excluded from both numerator and denominator.
gate_g4() {
  local mod="$1"
  local http_dir="$MODULES_DIR/$mod/src/http"
  if [ ! -d "$http_dir" ]; then
    echo "SKIP"
    return
  fi
  local utoipa_count=0 handler_count=0
  while IFS= read -r f; do
    if grep -q "#\[utoipa::path" "$f" 2>/dev/null; then
      local u h
      u=$(grep -c "#\[utoipa::path" "$f" 2>/dev/null || echo 0)
      h=$(grep -c "pub async fn" "$f" 2>/dev/null || echo 0)
      utoipa_count=$((utoipa_count + u))
      handler_count=$((handler_count + h))
    fi
  done < <(find "$http_dir" -name "*.rs" -type f 2>/dev/null)

  if [ "$handler_count" -eq 0 ]; then
    # Check main.rs or other handler locations
    handler_count=$(grep -c "pub async fn" "$MODULES_DIR/$mod/src/main.rs" 2>/dev/null || echo 0)
    utoipa_count=$(grep -c "#\[utoipa::path" "$MODULES_DIR/$mod/src/main.rs" 2>/dev/null || echo 0)
  fi

  if [ "$handler_count" -eq 0 ]; then
    echo "N/A"
    return
  fi

  local pct=$(( (utoipa_count * 100) / handler_count ))
  if [ "$pct" -ge 80 ]; then
    echo "PASS"
  elif in_g4_allowlist "$mod"; then
    echo "ALLOW($utoipa_count/$handler_count)"
  else
    echo "FAIL($utoipa_count/$handler_count)"
  fi
}

# G5: generated client crate exists
gate_g5() {
  local mod="$1"
  if [ -f "$CLIENTS_DIR/$mod/Cargo.toml" ]; then
    echo "PASS"
  else
    echo "FAIL"
  fi
}

# G6: auto-migrations configured
gate_g6() {
  local mod="$1"
  local main="$MODULES_DIR/$mod/src/main.rs"
  # Check for sqlx::migrate! macro or module.toml auto_migrate
  if grep -q 'sqlx::migrate!' "$main" 2>/dev/null; then
    echo "PASS"
    return
  fi
  if grep -q 'auto_migrate\s*=\s*true' "$MODULES_DIR/$mod/module.toml" 2>/dev/null; then
    echo "PASS"
    return
  fi
  # Check for MIGRATOR static
  if grep -q 'MIGRATOR' "$main" 2>/dev/null; then
    echo "PASS"
    return
  fi
  echo "FAIL"
}

# G7: event publish subjects well-formed (no double-prefix)
gate_g7() {
  local mod="$1"
  local toml="$MODULES_DIR/$mod/module.toml"
  # Only relevant if module has [bus] or [events.publish]
  if ! grep -q '\[bus\]\|\[events\.publish\]' "$toml" 2>/dev/null; then
    echo "N/A"
    return
  fi
  # Check allowlist — module's subject pattern documented as intentional
  if [ -f "$G7_ALLOWLIST" ] && grep -qE "^${mod}\b" "$G7_ALLOWLIST" 2>/dev/null; then
    echo "PASS"
    return
  fi
  local src_dir="$MODULES_DIR/$mod/src"
  local publisher_files
  publisher_files=$(find "$src_dir" -name "*.rs" -path "*/events/*" 2>/dev/null)
  if [ -z "$publisher_files" ]; then
    publisher_files=$(find "$src_dir" -name "outbox*.rs" -o -name "publisher*.rs" 2>/dev/null)
  fi
  if [ -z "$publisher_files" ]; then
    echo "N/A"
    return
  fi
  # Normalize module name (hyphens to underscores for subject matching)
  local mod_dot="${mod//-/_}"

  # Check if publisher adds prefix: format!("module.events.{}", event_type)
  local has_prefix_in_publisher=false
  if grep -rqE "format!\(\"${mod_dot}\.events\.\{" "$src_dir/events/" 2>/dev/null; then
    has_prefix_in_publisher=true
  fi

  if [ "$has_prefix_in_publisher" = true ]; then
    # Check if event_type CONSTANTS (not format strings) start with the module prefix.
    # Exclude the publisher format string itself to avoid false positives.
    if grep -rE "\"${mod_dot}\." "$src_dir/events/" 2>/dev/null \
       | grep -vE 'format!\(' \
       | grep -qE "\"${mod_dot}\."; then
      echo "FAIL"
      return
    fi
  fi
  echo "PASS"
}

# G8: consumer subscription subjects are valid
gate_g8() {
  local mod="$1"
  local main="$MODULES_DIR/$mod/src/main.rs"
  # Check for .consumer() calls
  if ! grep -q '\.consumer(' "$main" 2>/dev/null; then
    echo "N/A"
    return
  fi
  # Extract consumer subjects
  local subjects
  subjects=$(grep -oE '\.consumer\("([^"]+)"' "$main" 2>/dev/null | sed 's/\.consumer("//;s/"$//' || true)
  if [ -z "$subjects" ]; then
    # Multi-line consumer calls — extract subject from next line
    subjects=$(grep -A1 '\.consumer(' "$main" 2>/dev/null | grep -oE '"[a-z][a-z0-9_.]*"' | tr -d '"' || true)
  fi
  if [ -z "$subjects" ]; then
    echo "N/A"
    return
  fi
  # Validate: subjects should follow module.entity.action pattern
  local all_valid=true
  while IFS= read -r subj; do
    [ -z "$subj" ] && continue
    # Must have at least two dots: module.entity.action
    local dot_count
    dot_count=$(echo "$subj" | tr -cd '.' | wc -c | tr -d ' ')
    if [ "$dot_count" -lt 2 ]; then
      all_valid=false
    fi
  done <<< "$subjects"
  if [ "$all_valid" = true ]; then
    echo "PASS"
  else
    echo "FAIL"
  fi
}

# G9: health endpoints exist (/healthz, /api/health, /api/ready)
gate_g9() {
  local mod="$1"
  local src_dir="$MODULES_DIR/$mod/src"
  local main="$MODULES_DIR/$mod/src/main.rs"

  # ModuleBuilder provides /healthz, /api/health, /api/ready via platform SDK
  # unless skip_default_middleware is called.
  if grep -q 'ModuleBuilder' "$main" 2>/dev/null && \
     ! grep -q 'skip_default_middleware' "$main" 2>/dev/null; then
    echo "PASS"
    return
  fi

  # Fallback: check for explicit health references in module source
  local found=0
  grep -rql "healthz\|/api/health\b" "$src_dir" 2>/dev/null && found=$((found + 1))
  grep -rql "/api/ready" "$src_dir" 2>/dev/null && found=$((found + 1))
  if [ "$found" -ge 2 ]; then
    echo "PASS"
  elif [ "$found" -ge 1 ]; then
    echo "PARTIAL"
  else
    echo "FAIL"
  fi
}

# G10: no raw reqwest — should use PlatformClient/PlatformService
gate_g10() {
  local mod="$1"
  local src_dir="$MODULES_DIR/$mod/src"
  # Check for raw reqwest usage (excluding test files, Cargo.toml)
  local raw_reqwest
  raw_reqwest=$(grep -rn "reqwest::" "$src_dir" 2>/dev/null | grep -v "#\[cfg(test)\]\|#\[test\]\|/tests/\|test_" | wc -l | tr -d ' ')
  if [ "$raw_reqwest" -eq 0 ]; then
    echo "PASS"
  elif in_g10_allowlist "$mod"; then
    echo "ALLOW($raw_reqwest)"
  else
    echo "FAIL($raw_reqwest)"
  fi
}

# G11: all source files under 500 LOC (or in allowlist)
gate_g11() {
  local mod="$1"
  local src_dir="$MODULES_DIR/$mod/src"
  local violations=0
  while IFS= read -r file; do
    local lines
    lines=$(wc -l < "$file" | tr -d ' ')
    if [ "$lines" -gt 500 ]; then
      local relpath="${file#$PROJECT_ROOT/}"
      if ! in_allowlist "$relpath"; then
        violations=$((violations + 1))
      fi
    fi
  done < <(find "$src_dir" -name "*.rs" -type f 2>/dev/null)
  if [ "$violations" -eq 0 ]; then
    echo "PASS"
  else
    echo "FAIL($violations)"
  fi
}

# G12: module.toml exists with required sections
gate_g12() {
  local mod="$1"
  local toml="$MODULES_DIR/$mod/module.toml"
  if [ ! -f "$toml" ]; then
    echo "FAIL"
    return
  fi
  # Check for [module] section with name
  if grep -q '\[module\]' "$toml" 2>/dev/null && grep -q 'name\s*=' "$toml" 2>/dev/null; then
    echo "PASS"
  else
    echo "FAIL"
  fi
}

# ── Run all gates ────────────────────────────────────────────────

GATE_NAMES=("G1:Paginated" "G2:ApiError" "G3:utoipa" "G4:Coverage" "G5:Client" "G6:Migrate" "G7:PubSubj" "G8:ConSubj" "G9:Health" "G10:NoReqw" "G11:LOC" "G12:Manifest")
GATE_FUNCS=(gate_g1 gate_g2 gate_g3 gate_g4 gate_g5 gate_g6 gate_g7 gate_g8 gate_g9 gate_g10 gate_g11 gate_g12)

declare -A RESULTS
TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_CHECKS=0

for mod in "${MODULES[@]}"; do
  for i in "${!GATE_FUNCS[@]}"; do
    result=$("${GATE_FUNCS[$i]}" "$mod")
    RESULTS["$mod:$i"]="$result"
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    case "$result" in
      PASS|ALLOW*) TOTAL_PASS=$((TOTAL_PASS + 1)) ;;
      FAIL*) TOTAL_FAIL=$((TOTAL_FAIL + 1)) ;;
    esac
  done
done

# ── Output ───────────────────────────────────────────────────────

if $OUTPUT_JSON; then
  echo "{"
  echo "  \"timestamp\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
  echo "  \"git_sha\": \"$(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)\","
  echo "  \"total_modules\": ${#MODULES[@]},"
  echo "  \"total_checks\": $TOTAL_CHECKS,"
  echo "  \"total_pass\": $TOTAL_PASS,"
  echo "  \"total_fail\": $TOTAL_FAIL,"
  echo "  \"modules\": {"
  for idx in "${!MODULES[@]}"; do
    mod="${MODULES[$idx]}"
    echo -n "    \"$mod\": {"
    for i in "${!GATE_NAMES[@]}"; do
      gname="${GATE_NAMES[$i]}"
      result="${RESULTS[$mod:$i]}"
      echo -n "\"$gname\": \"$result\""
      [ "$i" -lt $((${#GATE_NAMES[@]} - 1)) ] && echo -n ", "
    done
    echo -n "}"
    [ "$idx" -lt $((${#MODULES[@]} - 1)) ] && echo "," || echo ""
  done
  echo "  }"
  echo "}"
  exit 0
fi

# Markdown table output
echo "# Plug-and-Play Gate Report"
echo ""
echo "**Date:** $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "**Git SHA:** $(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "**Modules:** ${#MODULES[@]} | **Checks:** $TOTAL_CHECKS | **Pass:** $TOTAL_PASS | **Fail:** $TOTAL_FAIL"
echo ""

# Header row
header="| Module |"
sep="|--------|"
for gname in "${GATE_NAMES[@]}"; do
  short="${gname#*:}"
  header="$header $short |"
  sep="$sep--------|"
done
echo "$header"
echo "$sep"

# Data rows
for mod in "${MODULES[@]}"; do
  row="| $mod |"
  for i in "${!GATE_NAMES[@]}"; do
    result="${RESULTS[$mod:$i]}"
    case "$result" in
      PASS)     cell="PASS" ;;
      ALLOW*)   cell="$result" ;;
      FAIL*)    cell="**$result**" ;;
      PARTIAL)  cell="~PARTIAL~" ;;
      SKIP)     cell="-" ;;
      N/A)      cell="-" ;;
      *)        cell="$result" ;;
    esac
    row="$row $cell |"
  done
  echo "$row"
done

echo ""
echo "## Legend"
echo ""
echo "| Gate | Description |"
echo "|------|-------------|"
echo "| G1:Paginated | PaginatedResponse used on list/search endpoints |"
echo "| G2:ApiError | ApiError with request_id on all error paths |"
echo "| G3:utoipa | utoipa annotations present in source |"
echo "| G4:Coverage | utoipa annotations >= 80% of handler fns (only files with utoipa count; repo/helper files excluded) |"
echo "| G5:Client | Generated client crate exists in clients/ |"
echo "| G6:Migrate | Auto-migrations configured (sqlx::migrate! or module.toml) |"
echo "| G7:PubSubj | Published event subjects well-formed (no double-prefix, or in .g7-subject-allowlist) |"
echo "| G8:ConSubj | Consumer subscription subjects are valid |"
echo "| G9:Health | Health endpoints present (via ModuleBuilder or explicit /healthz, /api/health, /api/ready) |"
echo "| G10:NoReqw | No raw reqwest usage (should use PlatformClient) |"
echo "| G11:LOC | All source files under 500 LOC (or in allowlist) |"
echo "| G12:Manifest | module.toml exists with required [module] section |"
echo ""
echo "**PASS** = gate satisfied | **FAIL(n)** = gate failed with n violations | **-** = not applicable"

# Exit code
[ "$TOTAL_FAIL" -eq 0 ] && exit 0 || exit 1
