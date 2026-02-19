#!/usr/bin/env bash
# dr_drill.sh — Quarterly disaster-recovery drill for the 7D Solutions Platform.
#
# Validates backup integrity, restore capability, database connectivity,
# NATS health, and service endpoints. Produces a timestamped report artifact.
#
# Usage:
#   bash scripts/dr_drill.sh [--dry-run]
#
# Options:
#   --dry-run    Print what the drill would do; make no changes
#
# Output:
#   dr-reports/dr-drill-YYYY-MM-DD_HH-MM-SS.txt  (timestamped report)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
DRY_RUN=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=true; shift ;;
    -*)        echo "Unknown option: $1" >&2; exit 1 ;;
    *)         echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

TIMESTAMP="$(date +%Y-%m-%d_%H-%M-%S)"
REPORT_DIR="${PROJECT_ROOT}/dr-reports"
REPORT_FILE="${REPORT_DIR}/dr-drill-${TIMESTAMP}.txt"

# ---------------------------------------------------------------------------
# Database catalogue: "name|host|port|db|user|pass"
# Mirrors docker-compose.infrastructure.yml defaults
# ---------------------------------------------------------------------------
DATABASES=(
  "auth|${AUTH_POSTGRES_HOST:-localhost}|${AUTH_POSTGRES_PORT:-5433}|${AUTH_POSTGRES_DB:-auth_db}|${AUTH_POSTGRES_USER:-auth_user}|${AUTH_POSTGRES_PASSWORD:-auth_pass}"
  "ar|${AR_POSTGRES_HOST:-localhost}|${AR_POSTGRES_PORT:-5434}|${AR_POSTGRES_DB:-ar_db}|${AR_POSTGRES_USER:-ar_user}|${AR_POSTGRES_PASSWORD:-ar_pass}"
  "subscriptions|${SUBSCRIPTIONS_POSTGRES_HOST:-localhost}|${SUBSCRIPTIONS_POSTGRES_PORT:-5435}|${SUBSCRIPTIONS_POSTGRES_DB:-subscriptions_db}|${SUBSCRIPTIONS_POSTGRES_USER:-subscriptions_user}|${SUBSCRIPTIONS_POSTGRES_PASSWORD:-subscriptions_pass}"
  "payments|${PAYMENTS_POSTGRES_HOST:-localhost}|${PAYMENTS_POSTGRES_PORT:-5436}|${PAYMENTS_POSTGRES_DB:-payments_db}|${PAYMENTS_POSTGRES_USER:-payments_user}|${PAYMENTS_POSTGRES_PASSWORD:-payments_pass}"
  "notifications|${NOTIFICATIONS_POSTGRES_HOST:-localhost}|${NOTIFICATIONS_POSTGRES_PORT:-5437}|${NOTIFICATIONS_POSTGRES_DB:-notifications_db}|${NOTIFICATIONS_POSTGRES_USER:-notifications_user}|${NOTIFICATIONS_POSTGRES_PASSWORD:-notifications_pass}"
  "gl|${GL_POSTGRES_HOST:-localhost}|${GL_POSTGRES_PORT:-5438}|${GL_POSTGRES_DB:-gl_db}|${GL_POSTGRES_USER:-gl_user}|${GL_POSTGRES_PASSWORD:-gl_pass}"
  "projections|${PROJECTIONS_POSTGRES_HOST:-localhost}|${PROJECTIONS_POSTGRES_PORT:-5439}|${PROJECTIONS_POSTGRES_DB:-projections_db}|${PROJECTIONS_POSTGRES_USER:-projections_user}|${PROJECTIONS_POSTGRES_PASSWORD:-projections_pass}"
  "audit|${AUDIT_POSTGRES_HOST:-localhost}|${AUDIT_POSTGRES_PORT:-5440}|${AUDIT_POSTGRES_DB:-audit_db}|${AUDIT_POSTGRES_USER:-audit_user}|${AUDIT_POSTGRES_PASSWORD:-audit_pass}"
  "tenant_registry|${TENANT_REGISTRY_POSTGRES_HOST:-localhost}|${TENANT_REGISTRY_POSTGRES_PORT:-5441}|${TENANT_REGISTRY_POSTGRES_DB:-tenant_registry_db}|${TENANT_REGISTRY_POSTGRES_USER:-tenant_registry_user}|${TENANT_REGISTRY_POSTGRES_PASSWORD:-tenant_registry_pass}"
  "inventory|${INVENTORY_POSTGRES_HOST:-localhost}|${INVENTORY_POSTGRES_PORT:-5442}|${INVENTORY_POSTGRES_DB:-inventory_db}|${INVENTORY_POSTGRES_USER:-inventory_user}|${INVENTORY_POSTGRES_PASSWORD:-inventory_pass}"
  "ap|${AP_POSTGRES_HOST:-localhost}|${AP_POSTGRES_PORT:-5443}|${AP_POSTGRES_DB:-ap_db}|${AP_POSTGRES_USER:-ap_user}|${AP_POSTGRES_PASSWORD:-ap_pass}"
  "treasury|${TREASURY_POSTGRES_HOST:-localhost}|${TREASURY_POSTGRES_PORT:-5444}|${TREASURY_POSTGRES_DB:-treasury_db}|${TREASURY_POSTGRES_USER:-treasury_user}|${TREASURY_POSTGRES_PASSWORD:-treasury_pass}"
  "fixed_assets|${FIXED_ASSETS_POSTGRES_HOST:-localhost}|${FIXED_ASSETS_POSTGRES_PORT:-5445}|${FIXED_ASSETS_POSTGRES_DB:-fixed_assets_db}|${FIXED_ASSETS_POSTGRES_USER:-fixed_assets_user}|${FIXED_ASSETS_POSTGRES_PASSWORD:-fixed_assets_pass}"
  "consolidation|${CONSOLIDATION_POSTGRES_HOST:-localhost}|${CONSOLIDATION_POSTGRES_PORT:-5446}|${CONSOLIDATION_POSTGRES_DB:-consolidation_db}|${CONSOLIDATION_POSTGRES_USER:-consolidation_user}|${CONSOLIDATION_POSTGRES_PASSWORD:-consolidation_pass}"
  "timekeeping|${TIMEKEEPING_POSTGRES_HOST:-localhost}|${TIMEKEEPING_POSTGRES_PORT:-5447}|${TIMEKEEPING_POSTGRES_DB:-timekeeping_db}|${TIMEKEEPING_POSTGRES_USER:-timekeeping_user}|${TIMEKEEPING_POSTGRES_PASSWORD:-timekeeping_pass}"
  "party|${PARTY_POSTGRES_HOST:-localhost}|${PARTY_POSTGRES_PORT:-5448}|${PARTY_POSTGRES_DB:-party_db}|${PARTY_POSTGRES_USER:-party_user}|${PARTY_POSTGRES_PASSWORD:-party_pass}"
  "integrations|${INTEGRATIONS_POSTGRES_HOST:-localhost}|${INTEGRATIONS_POSTGRES_PORT:-5449}|${INTEGRATIONS_POSTGRES_DB:-integrations_db}|${INTEGRATIONS_POSTGRES_USER:-integrations_user}|${INTEGRATIONS_POSTGRES_PASSWORD:-integrations_pass}"
)

# Service health ports (module services)
SERVICE_PORTS=(8081 8082 8083 8084 8085 8086 8087 8088 8089 8090 8091 8092 8093 8094 8095 8096 8097 8098 8099)

# NATS default
NATS_URL="${NATS_URL:-localhost:4222}"
NATS_MONITOR="${NATS_MONITOR:-localhost:8222}"

# ---------------------------------------------------------------------------
# Dry-run: show plan and exit
# ---------------------------------------------------------------------------
if $DRY_RUN; then
  echo "=== DRY RUN: dr_drill.sh ==="
  echo "Timestamp : ${TIMESTAMP}"
  echo "Report    : ${REPORT_FILE}"
  echo ""
  echo "Steps the drill would execute:"
  echo "  1. Check connectivity to ${#DATABASES[@]} PostgreSQL databases"
  echo "  2. Create a fresh backup via scripts/backup_all.sh"
  echo "  3. Verify backup integrity (gzip + manifest)"
  echo "  4. Run smoke-test verification (row counts vs manifest)"
  echo "  5. Check NATS connectivity at ${NATS_URL}"
  echo "  6. Check service health on ports: ${SERVICE_PORTS[*]}"
  echo "  7. Produce timestamped report: ${REPORT_FILE}"
  echo ""
  echo "Databases:"
  for entry in "${DATABASES[@]}"; do
    IFS='|' read -r name host port db user _pass <<< "$entry"
    printf "  %-20s %s@%s:%s/%s\n" "${name}" "${user}" "${host}" "${port}" "${db}"
  done
  echo ""
  echo "=== DRY RUN complete. No changes made. ==="
  exit 0
fi

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------
mkdir -p "${REPORT_DIR}"

# Counters
TOTAL_CHECKS=0
PASS_CHECKS=0
FAIL_CHECKS=0
WARN_CHECKS=0

# Report is built in memory and written at the end
REPORT_LINES=()

report() {
  local line="[$(date +%H:%M:%S)] $*"
  REPORT_LINES+=("$line")
  echo "$line"
}

check_pass() {
  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  PASS_CHECKS=$((PASS_CHECKS + 1))
  report "PASS  $*"
}

check_fail() {
  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  FAIL_CHECKS=$((FAIL_CHECKS + 1))
  report "FAIL  $*"
}

check_warn() {
  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  WARN_CHECKS=$((WARN_CHECKS + 1))
  report "WARN  $*"
}

# ---------------------------------------------------------------------------
# Header
# ---------------------------------------------------------------------------
report "=============================================="
report "  7D Solutions Platform — DR Drill Report"
report "=============================================="
report "Timestamp : ${TIMESTAMP}"
report "Hostname  : $(hostname)"
report "Operator  : ${USER:-unknown}"
report ""

# ---------------------------------------------------------------------------
# Step 1: Database Connectivity
# ---------------------------------------------------------------------------
report "--- Step 1: Database Connectivity (${#DATABASES[@]} databases) ---"
DB_REACHABLE=0
DB_UNREACHABLE=0

for entry in "${DATABASES[@]}"; do
  IFS='|' read -r name host port db user pass <<< "$entry"
  if PGPASSWORD="$pass" pg_isready -h "$host" -p "$port" -U "$user" -d "$db" -q 2>/dev/null; then
    check_pass "${name}: reachable at ${host}:${port}"
    DB_REACHABLE=$((DB_REACHABLE + 1))
  else
    check_fail "${name}: NOT reachable at ${host}:${port}"
    DB_UNREACHABLE=$((DB_UNREACHABLE + 1))
  fi
done

report "Databases reachable: ${DB_REACHABLE}/${#DATABASES[@]}"
report ""

# ---------------------------------------------------------------------------
# Step 2: Fresh Backup
# ---------------------------------------------------------------------------
report "--- Step 2: Create Fresh Backup ---"
DRILL_BACKUP_DIR="${PROJECT_ROOT}/backups/dr-drill-${TIMESTAMP}"

if bash "${SCRIPT_DIR}/backup_all.sh" "${DRILL_BACKUP_DIR}" 2>&1; then
  check_pass "Backup completed: ${DRILL_BACKUP_DIR}"
else
  check_fail "Backup script returned non-zero exit code"
fi
report ""

# ---------------------------------------------------------------------------
# Step 3: Verify Backup Integrity
# ---------------------------------------------------------------------------
report "--- Step 3: Backup Integrity Verification ---"

MANIFEST="${DRILL_BACKUP_DIR}/manifest.json"
if [[ -f "$MANIFEST" ]]; then
  check_pass "manifest.json exists"

  # Validate JSON structure
  if python3 -c "import json; json.load(open('${MANIFEST}'))" 2>/dev/null; then
    check_pass "manifest.json is valid JSON"

    # Check error count in manifest
    MANIFEST_ERRORS="$(python3 -c "import json; print(json.load(open('${MANIFEST}'))['errors'])" 2>/dev/null)"
    if [[ "$MANIFEST_ERRORS" == "0" ]]; then
      check_pass "Manifest reports 0 errors"
    else
      check_fail "Manifest reports ${MANIFEST_ERRORS} error(s)"
    fi

    # Count databases in manifest
    MANIFEST_DB_COUNT="$(python3 -c "import json; print(len(json.load(open('${MANIFEST}'))['databases']))" 2>/dev/null)"
    report "INFO  Manifest contains ${MANIFEST_DB_COUNT} database(s)"
  else
    check_fail "manifest.json is not valid JSON"
  fi
else
  check_fail "manifest.json not found in backup directory"
fi

# Verify gzip integrity of each dump
GZIP_OK=0
GZIP_FAIL=0
for f in "${DRILL_BACKUP_DIR}"/*.sql.gz; do
  [[ -f "$f" ]] || continue
  BASENAME="$(basename "$f")"
  if gunzip -t "$f" 2>/dev/null; then
    check_pass "gzip OK: ${BASENAME}"
    GZIP_OK=$((GZIP_OK + 1))
  else
    check_fail "gzip CORRUPT: ${BASENAME}"
    GZIP_FAIL=$((GZIP_FAIL + 1))
  fi
done

report "Gzip integrity: ${GZIP_OK} OK, ${GZIP_FAIL} corrupt"
report ""

# ---------------------------------------------------------------------------
# Step 4: Smoke-Test (row count verification)
# ---------------------------------------------------------------------------
report "--- Step 4: Smoke-Test Verification ---"

if [[ -f "$MANIFEST" ]]; then
  SMOKE_OUTPUT="$(bash "${SCRIPT_DIR}/restore_all.sh" "${DRILL_BACKUP_DIR}" --smoke-test 2>&1)" || true
  SMOKE_PASS="$(echo "$SMOKE_OUTPUT" | grep -c "^.*PASS" || true)"
  SMOKE_FAIL="$(echo "$SMOKE_OUTPUT" | grep -c "^.*FAIL" || true)"

  if echo "$SMOKE_OUTPUT" | grep -q "RESTORE OK"; then
    check_pass "Smoke test: all row counts match manifest"
  else
    check_fail "Smoke test: ${SMOKE_FAIL} failure(s)"
  fi

  report "INFO  Smoke-test detail: ${SMOKE_PASS} pass, ${SMOKE_FAIL} fail"
else
  check_fail "Skipped smoke test — no manifest available"
fi
report ""

# ---------------------------------------------------------------------------
# Step 5: NATS Connectivity
# ---------------------------------------------------------------------------
report "--- Step 5: NATS Connectivity ---"

# Check NATS monitoring endpoint
if curl -sf "http://${NATS_MONITOR}/healthz" >/dev/null 2>&1; then
  check_pass "NATS monitoring endpoint responsive at ${NATS_MONITOR}"
else
  check_warn "NATS monitoring endpoint not reachable at ${NATS_MONITOR}"
fi

# Check NATS JetStream via monitoring API
JS_INFO="$(curl -sf "http://${NATS_MONITOR}/jsz" 2>/dev/null)" || JS_INFO=""
if [[ -n "$JS_INFO" ]]; then
  JS_STREAMS="$(echo "$JS_INFO" | python3 -c "import json,sys; print(json.load(sys.stdin).get('streams',0))" 2>/dev/null || echo "?")"
  check_pass "NATS JetStream active: ${JS_STREAMS} stream(s)"
else
  check_warn "NATS JetStream info not available"
fi
report ""

# ---------------------------------------------------------------------------
# Step 6: Service Health Endpoints
# ---------------------------------------------------------------------------
report "--- Step 6: Service Health Endpoints ---"
SVC_UP=0
SVC_DOWN=0

for port in "${SERVICE_PORTS[@]}"; do
  if curl -sf "http://localhost:${port}/health" >/dev/null 2>&1; then
    check_pass "Service on :${port} healthy"
    SVC_UP=$((SVC_UP + 1))
  else
    check_warn "Service on :${port} not responding"
    SVC_DOWN=$((SVC_DOWN + 1))
  fi
done

report "Services healthy: ${SVC_UP}/${#SERVICE_PORTS[@]}"
report ""

# ---------------------------------------------------------------------------
# Step 7: RPO Assessment
# ---------------------------------------------------------------------------
report "--- Step 7: RPO / Backup Freshness ---"

# Find the most recent backup BEFORE the drill backup
LATEST_BACKUP=""
for d in $(ls -1td "${PROJECT_ROOT}"/backups/*/ 2>/dev/null); do
  # Skip the drill backup itself
  [[ "$d" == "${DRILL_BACKUP_DIR}/" ]] && continue
  LATEST_BACKUP="$d"
  break
done

if [[ -n "$LATEST_BACKUP" ]]; then
  BACKUP_MANIFEST="${LATEST_BACKUP}manifest.json"
  if [[ -f "$BACKUP_MANIFEST" ]]; then
    BACKUP_TS="$(python3 -c "import json; print(json.load(open('${BACKUP_MANIFEST}'))['timestamp'])" 2>/dev/null || echo "unknown")"
    report "INFO  Most recent pre-drill backup: ${BACKUP_TS}"
    report "INFO  Location: ${LATEST_BACKUP}"

    # Calculate age in hours (rough)
    BACKUP_DATE="${BACKUP_TS%%_*}"
    if [[ "$BACKUP_DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
      BACKUP_EPOCH="$(date -j -f "%Y-%m-%d" "$BACKUP_DATE" "+%s" 2>/dev/null || date -d "$BACKUP_DATE" "+%s" 2>/dev/null || echo "")"
      NOW_EPOCH="$(date "+%s")"
      if [[ -n "$BACKUP_EPOCH" ]]; then
        AGE_HOURS=$(( (NOW_EPOCH - BACKUP_EPOCH) / 3600 ))
        if [[ $AGE_HOURS -le 1 ]]; then
          check_pass "Backup age: ${AGE_HOURS}h (within 1h RPO for critical tier)"
        elif [[ $AGE_HOURS -le 4 ]]; then
          check_warn "Backup age: ${AGE_HOURS}h (within 4h RPO for standard tier; exceeds critical)"
        else
          check_fail "Backup age: ${AGE_HOURS}h (exceeds all RPO targets)"
        fi
      else
        check_warn "Could not compute backup age"
      fi
    fi
  else
    check_warn "Pre-drill backup has no manifest"
  fi
else
  check_warn "No pre-drill backups found (this drill created the first)"
fi
report ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
report "=============================================="
report "  DR Drill Summary"
report "=============================================="
report "Total checks : ${TOTAL_CHECKS}"
report "Passed       : ${PASS_CHECKS}"
report "Warnings     : ${WARN_CHECKS}"
report "Failed       : ${FAIL_CHECKS}"
report ""

if [[ $FAIL_CHECKS -eq 0 && $WARN_CHECKS -eq 0 ]]; then
  VERDICT="DRILL PASSED — all checks green"
elif [[ $FAIL_CHECKS -eq 0 ]]; then
  VERDICT="DRILL PASSED WITH WARNINGS — review warnings above"
else
  VERDICT="DRILL FAILED — ${FAIL_CHECKS} check(s) failed"
fi

report "${VERDICT}"
report ""
report "Report: ${REPORT_FILE}"
report "Backup: ${DRILL_BACKUP_DIR}"

# ---------------------------------------------------------------------------
# Write report to file
# ---------------------------------------------------------------------------
{
  for line in "${REPORT_LINES[@]}"; do
    echo "$line"
  done
} > "${REPORT_FILE}"

echo ""
echo "Report written to: ${REPORT_FILE}"

# Exit with failure if any checks failed
if [[ $FAIL_CHECKS -gt 0 ]]; then
  exit 1
fi
exit 0
