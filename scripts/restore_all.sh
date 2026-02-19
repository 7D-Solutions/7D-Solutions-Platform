#!/usr/bin/env bash
# restore_all.sh — restore all module databases from a backup directory.
#
# Usage:
#   ./scripts/restore_all.sh <BACKUP_DIR> [--smoke-test]
#
# Options:
#   --smoke-test   Verify row counts against manifest only; do NOT restore
#
# The script reads manifest.json from BACKUP_DIR to discover which databases
# to restore and what row counts to expect.  On a full restore it:
#   1. Drops the target database (if exists)
#   2. Creates a fresh database
#   3. Pipes the .sql.gz dump through psql
#   4. Runs a smoke test to confirm row counts match the manifest
#
# Environment overrides (all have sane defaults matching docker-compose.infrastructure.yml):
#   AUTH_POSTGRES_HOST, AUTH_POSTGRES_PORT, AUTH_POSTGRES_USER, ...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
SMOKE_TEST_ONLY=false
BACKUP_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --smoke-test) SMOKE_TEST_ONLY=true; shift ;;
    -*)           echo "Unknown option: $1" >&2; exit 1 ;;
    *)            BACKUP_DIR="$1"; shift ;;
  esac
done

if [[ -z "$BACKUP_DIR" ]]; then
  echo "Usage: $0 <BACKUP_DIR> [--smoke-test]" >&2
  exit 1
fi

if [[ ! -d "$BACKUP_DIR" ]]; then
  echo "ERROR: Backup directory not found: ${BACKUP_DIR}" >&2
  exit 1
fi

MANIFEST="${BACKUP_DIR}/manifest.json"
if [[ ! -f "$MANIFEST" ]]; then
  echo "ERROR: manifest.json not found in ${BACKUP_DIR}" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Connection overrides — mirrors docker-compose.infrastructure.yml defaults.
# Restore always connects via the module's own user; the DROP/CREATE step
# requires a superuser which defaults to postgres on port 5432.
# ---------------------------------------------------------------------------
SUPERUSER_HOST="${SUPERUSER_POSTGRES_HOST:-localhost}"
SUPERUSER_PORT="${SUPERUSER_POSTGRES_PORT:-5432}"
SUPERUSER_USER="${SUPERUSER_POSTGRES_USER:-postgres}"
SUPERUSER_PASS="${SUPERUSER_POSTGRES_PASSWORD:-postgres}"

# Per-module credential overrides (mirrors docker-compose defaults)
declare -A MODULE_HOST MODULE_PORT MODULE_DB MODULE_USER MODULE_PASS
MODULE_HOST[auth]="${AUTH_POSTGRES_HOST:-localhost}"
MODULE_PORT[auth]="${AUTH_POSTGRES_PORT:-5433}"
MODULE_DB[auth]="${AUTH_POSTGRES_DB:-auth_db}"
MODULE_USER[auth]="${AUTH_POSTGRES_USER:-auth_user}"
MODULE_PASS[auth]="${AUTH_POSTGRES_PASSWORD:-auth_pass}"

MODULE_HOST[ar]="${AR_POSTGRES_HOST:-localhost}"
MODULE_PORT[ar]="${AR_POSTGRES_PORT:-5434}"
MODULE_DB[ar]="${AR_POSTGRES_DB:-ar_db}"
MODULE_USER[ar]="${AR_POSTGRES_USER:-ar_user}"
MODULE_PASS[ar]="${AR_POSTGRES_PASSWORD:-ar_pass}"

MODULE_HOST[subscriptions]="${SUBSCRIPTIONS_POSTGRES_HOST:-localhost}"
MODULE_PORT[subscriptions]="${SUBSCRIPTIONS_POSTGRES_PORT:-5435}"
MODULE_DB[subscriptions]="${SUBSCRIPTIONS_POSTGRES_DB:-subscriptions_db}"
MODULE_USER[subscriptions]="${SUBSCRIPTIONS_POSTGRES_USER:-subscriptions_user}"
MODULE_PASS[subscriptions]="${SUBSCRIPTIONS_POSTGRES_PASSWORD:-subscriptions_pass}"

MODULE_HOST[payments]="${PAYMENTS_POSTGRES_HOST:-localhost}"
MODULE_PORT[payments]="${PAYMENTS_POSTGRES_PORT:-5436}"
MODULE_DB[payments]="${PAYMENTS_POSTGRES_DB:-payments_db}"
MODULE_USER[payments]="${PAYMENTS_POSTGRES_USER:-payments_user}"
MODULE_PASS[payments]="${PAYMENTS_POSTGRES_PASSWORD:-payments_pass}"

MODULE_HOST[notifications]="${NOTIFICATIONS_POSTGRES_HOST:-localhost}"
MODULE_PORT[notifications]="${NOTIFICATIONS_POSTGRES_PORT:-5437}"
MODULE_DB[notifications]="${NOTIFICATIONS_POSTGRES_DB:-notifications_db}"
MODULE_USER[notifications]="${NOTIFICATIONS_POSTGRES_USER:-notifications_user}"
MODULE_PASS[notifications]="${NOTIFICATIONS_POSTGRES_PASSWORD:-notifications_pass}"

MODULE_HOST[gl]="${GL_POSTGRES_HOST:-localhost}"
MODULE_PORT[gl]="${GL_POSTGRES_PORT:-5438}"
MODULE_DB[gl]="${GL_POSTGRES_DB:-gl_db}"
MODULE_USER[gl]="${GL_POSTGRES_USER:-gl_user}"
MODULE_PASS[gl]="${GL_POSTGRES_PASSWORD:-gl_pass}"

MODULE_HOST[projections]="${PROJECTIONS_POSTGRES_HOST:-localhost}"
MODULE_PORT[projections]="${PROJECTIONS_POSTGRES_PORT:-5439}"
MODULE_DB[projections]="${PROJECTIONS_POSTGRES_DB:-projections_db}"
MODULE_USER[projections]="${PROJECTIONS_POSTGRES_USER:-projections_user}"
MODULE_PASS[projections]="${PROJECTIONS_POSTGRES_PASSWORD:-projections_pass}"

MODULE_HOST[audit]="${AUDIT_POSTGRES_HOST:-localhost}"
MODULE_PORT[audit]="${AUDIT_POSTGRES_PORT:-5440}"
MODULE_DB[audit]="${AUDIT_POSTGRES_DB:-audit_db}"
MODULE_USER[audit]="${AUDIT_POSTGRES_USER:-audit_user}"
MODULE_PASS[audit]="${AUDIT_POSTGRES_PASSWORD:-audit_pass}"

MODULE_HOST[tenant_registry]="${TENANT_REGISTRY_POSTGRES_HOST:-localhost}"
MODULE_PORT[tenant_registry]="${TENANT_REGISTRY_POSTGRES_PORT:-5441}"
MODULE_DB[tenant_registry]="${TENANT_REGISTRY_POSTGRES_DB:-tenant_registry_db}"
MODULE_USER[tenant_registry]="${TENANT_REGISTRY_POSTGRES_USER:-tenant_registry_user}"
MODULE_PASS[tenant_registry]="${TENANT_REGISTRY_POSTGRES_PASSWORD:-tenant_registry_pass}"

MODULE_HOST[inventory]="${INVENTORY_POSTGRES_HOST:-localhost}"
MODULE_PORT[inventory]="${INVENTORY_POSTGRES_PORT:-5442}"
MODULE_DB[inventory]="${INVENTORY_POSTGRES_DB:-inventory_db}"
MODULE_USER[inventory]="${INVENTORY_POSTGRES_USER:-inventory_user}"
MODULE_PASS[inventory]="${INVENTORY_POSTGRES_PASSWORD:-inventory_pass}"

MODULE_HOST[ap]="${AP_POSTGRES_HOST:-localhost}"
MODULE_PORT[ap]="${AP_POSTGRES_PORT:-5443}"
MODULE_DB[ap]="${AP_POSTGRES_DB:-ap_db}"
MODULE_USER[ap]="${AP_POSTGRES_USER:-ap_user}"
MODULE_PASS[ap]="${AP_POSTGRES_PASSWORD:-ap_pass}"

MODULE_HOST[treasury]="${TREASURY_POSTGRES_HOST:-localhost}"
MODULE_PORT[treasury]="${TREASURY_POSTGRES_PORT:-5444}"
MODULE_DB[treasury]="${TREASURY_POSTGRES_DB:-treasury_db}"
MODULE_USER[treasury]="${TREASURY_POSTGRES_USER:-treasury_user}"
MODULE_PASS[treasury]="${TREASURY_POSTGRES_PASSWORD:-treasury_pass}"

MODULE_HOST[fixed_assets]="${FIXED_ASSETS_POSTGRES_HOST:-localhost}"
MODULE_PORT[fixed_assets]="${FIXED_ASSETS_POSTGRES_PORT:-5445}"
MODULE_DB[fixed_assets]="${FIXED_ASSETS_POSTGRES_DB:-fixed_assets_db}"
MODULE_USER[fixed_assets]="${FIXED_ASSETS_POSTGRES_USER:-fixed_assets_user}"
MODULE_PASS[fixed_assets]="${FIXED_ASSETS_POSTGRES_PASSWORD:-fixed_assets_pass}"

MODULE_HOST[consolidation]="${CONSOLIDATION_POSTGRES_HOST:-localhost}"
MODULE_PORT[consolidation]="${CONSOLIDATION_POSTGRES_PORT:-5446}"
MODULE_DB[consolidation]="${CONSOLIDATION_POSTGRES_DB:-consolidation_db}"
MODULE_USER[consolidation]="${CONSOLIDATION_POSTGRES_USER:-consolidation_user}"
MODULE_PASS[consolidation]="${CONSOLIDATION_POSTGRES_PASSWORD:-consolidation_pass}"

MODULE_HOST[timekeeping]="${TIMEKEEPING_POSTGRES_HOST:-localhost}"
MODULE_PORT[timekeeping]="${TIMEKEEPING_POSTGRES_PORT:-5447}"
MODULE_DB[timekeeping]="${TIMEKEEPING_POSTGRES_DB:-timekeeping_db}"
MODULE_USER[timekeeping]="${TIMEKEEPING_POSTGRES_USER:-timekeeping_user}"
MODULE_PASS[timekeeping]="${TIMEKEEPING_POSTGRES_PASSWORD:-timekeeping_pass}"

MODULE_HOST[party]="${PARTY_POSTGRES_HOST:-localhost}"
MODULE_PORT[party]="${PARTY_POSTGRES_PORT:-5448}"
MODULE_DB[party]="${PARTY_POSTGRES_DB:-party_db}"
MODULE_USER[party]="${PARTY_POSTGRES_USER:-party_user}"
MODULE_PASS[party]="${PARTY_POSTGRES_PASSWORD:-party_pass}"

MODULE_HOST[integrations]="${INTEGRATIONS_POSTGRES_HOST:-localhost}"
MODULE_PORT[integrations]="${INTEGRATIONS_POSTGRES_PORT:-5449}"
MODULE_DB[integrations]="${INTEGRATIONS_POSTGRES_DB:-integrations_db}"
MODULE_USER[integrations]="${INTEGRATIONS_POSTGRES_USER:-integrations_user}"
MODULE_PASS[integrations]="${INTEGRATIONS_POSTGRES_PASSWORD:-integrations_pass}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log() { echo "[$(date +%H:%M:%S)] $*"; }

# Extract a field from manifest.json using only sh builtins + grep + sed.
# Usage: manifest_field <name> <field>    e.g. manifest_field ar file
manifest_field() {
  local block field value
  # Pull the JSON object for this database name
  block="$(python3 -c "
import json, sys
data = json.load(open('${MANIFEST}'))
for db in data['databases']:
    if db['name'] == '$1':
        print(json.dumps(db))
        sys.exit(0)
sys.exit(1)
" 2>/dev/null)" || return 1
  python3 -c "import json, sys; d=json.loads(sys.stdin.read()); print(d['$2'])" <<< "$block"
}

# Return a newline-separated list of "table count" pairs from manifest
manifest_tables() {
  python3 -c "
import json, sys
data = json.load(open('${MANIFEST}'))
for db in data['databases']:
    if db['name'] == '$1':
        for t in db.get('tables', []):
            print(t['table'] + ' ' + str(t['count']))
        sys.exit(0)
sys.exit(1)
" 2>/dev/null
}

# Count rows in a single table
table_count() {
  local host="$1" port="$2" db="$3" user="$4" pass="$5" table="$6"
  PGPASSWORD="$pass" psql -h "$host" -p "$port" -U "$user" -d "$db" -t -A \
    -c "SELECT COUNT(*) FROM \"${table}\";" 2>/dev/null | tr -d ' '
}

# ---------------------------------------------------------------------------
# Read database names from manifest
# ---------------------------------------------------------------------------
NAMES=($(python3 -c "
import json
data = json.load(open('${MANIFEST}'))
for db in data['databases']:
    print(db['name'])
"))

log "Manifest: ${MANIFEST}"
log "Databases in manifest: ${#NAMES[@]}"
log "Mode: $(${SMOKE_TEST_ONLY} && echo 'smoke-test only' || echo 'full restore')"

ERRORS=0

# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------
for name in "${NAMES[@]}"; do
  host="${MODULE_HOST[$name]:-localhost}"
  port="${MODULE_PORT[$name]:-5432}"
  db="${MODULE_DB[$name]:-${name}_db}"
  user="${MODULE_USER[$name]:-${name}_user}"
  pass="${MODULE_PASS[$name]:-${name}_pass}"

  DUMP_FILE="${BACKUP_DIR}/${name}.sql.gz"

  # ------------------------------------------------------------------
  # Full restore path
  # ------------------------------------------------------------------
  if ! $SMOKE_TEST_ONLY; then
    if [[ ! -f "$DUMP_FILE" ]]; then
      log "WARN  ${name}: dump file not found (${DUMP_FILE}) — skipping"
      ERRORS=$((ERRORS + 1))
      continue
    fi

    # Verify gzip integrity before touching the live DB
    if ! gunzip -t "${DUMP_FILE}" 2>/dev/null; then
      log "ERROR ${name}: dump file is corrupt — skipping"
      ERRORS=$((ERRORS + 1))
      continue
    fi

    log "INFO  ${name}: dropping and recreating ${db}"
    PGPASSWORD="$SUPERUSER_PASS" psql \
      -h "$SUPERUSER_HOST" -p "$SUPERUSER_PORT" -U "$SUPERUSER_USER" \
      -c "DROP DATABASE IF EXISTS \"${db}\";" 2>/dev/null || true
    PGPASSWORD="$SUPERUSER_PASS" psql \
      -h "$SUPERUSER_HOST" -p "$SUPERUSER_PORT" -U "$SUPERUSER_USER" \
      -c "CREATE DATABASE \"${db}\" OWNER \"${user}\";" 2>/dev/null || \
    PGPASSWORD="$SUPERUSER_PASS" psql \
      -h "$SUPERUSER_HOST" -p "$SUPERUSER_PORT" -U "$SUPERUSER_USER" \
      -c "CREATE DATABASE \"${db}\";" 2>/dev/null

    log "INFO  ${name}: restoring from ${name}.sql.gz"
    if ! gunzip -c "${DUMP_FILE}" | \
        PGPASSWORD="$pass" psql -h "$host" -p "$port" -U "$user" -d "$db" \
          --quiet 2>/dev/null; then
      # Fallback: try superuser credentials (dump may have role-specific objects)
      log "WARN  ${name}: restore with module user failed; retrying with superuser"
      gunzip -c "${DUMP_FILE}" | \
        PGPASSWORD="$SUPERUSER_PASS" psql \
          -h "$SUPERUSER_HOST" -p "$SUPERUSER_PORT" -U "$SUPERUSER_USER" \
          -d "$db" --quiet 2>/dev/null || {
        log "ERROR ${name}: restore failed"
        ERRORS=$((ERRORS + 1))
        continue
      }
    fi
    log "INFO  ${name}: restore complete"
  fi

  # ------------------------------------------------------------------
  # Smoke test: verify row counts against manifest
  # ------------------------------------------------------------------
  log "INFO  ${name}: running smoke test"
  DB_ERRORS=0

  while IFS=' ' read -r tbl expected; do
    [[ -z "$tbl" ]] && continue
    actual="$(table_count "$host" "$port" "$db" "$user" "$pass" "$tbl" 2>/dev/null || echo "ERR")"
    if [[ "$actual" == "ERR" || -z "$actual" ]]; then
      log "WARN  ${name}.${tbl}: could not query count (table may not exist)"
      DB_ERRORS=$((DB_ERRORS + 1))
    elif [[ "$actual" != "$expected" ]]; then
      log "FAIL  ${name}.${tbl}: expected ${expected} rows, got ${actual}"
      DB_ERRORS=$((DB_ERRORS + 1))
    else
      log "PASS  ${name}.${tbl}: ${actual} rows OK"
    fi
  done < <(manifest_tables "$name")

  if [[ $DB_ERRORS -eq 0 ]]; then
    log "PASS  ${name}: all table counts match manifest"
  else
    log "FAIL  ${name}: ${DB_ERRORS} table(s) failed smoke test"
    ERRORS=$((ERRORS + DB_ERRORS))
  fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if [[ $ERRORS -eq 0 ]]; then
  echo "=== RESTORE OK: all databases verified against manifest ==="
  exit 0
else
  echo "=== RESTORE FAILED: ${ERRORS} error(s) — review output above ==="
  exit 1
fi
