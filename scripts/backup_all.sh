#!/usr/bin/env bash
# backup_all.sh — dump all module PostgreSQL databases to a timestamped directory.
#
# Usage:
#   ./scripts/backup_all.sh [--dry-run] [BACKUP_DIR]
#
# Options:
#   --dry-run    Print what would happen; make no changes
#   BACKUP_DIR   Optional target directory (default: ./backups/YYYY-MM-DD_HH-MM-SS)
#
# Environment overrides (all have sane defaults matching docker-compose.infrastructure.yml):
#   AUTH_POSTGRES_HOST, AUTH_POSTGRES_PORT, AUTH_POSTGRES_DB, ...
#
# Outputs (in BACKUP_DIR):
#   <module>.sql.gz   — compressed pg_dump for each database
#   manifest.json     — metadata with row-count snapshot per table
#   backup.log        — timestamped execution log

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
DRY_RUN=false
BACKUP_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=true; shift ;;
    -*)        echo "Unknown option: $1" >&2; exit 1 ;;
    *)         BACKUP_DIR="$1"; shift ;;
  esac
done

TIMESTAMP="$(date +%Y-%m-%d_%H-%M-%S)"
[[ -z "$BACKUP_DIR" ]] && BACKUP_DIR="${PROJECT_ROOT}/backups/${TIMESTAMP}"

# ---------------------------------------------------------------------------
# Database catalogue: "name|host|port|db|user|pass"
# Defaults match docker-compose.infrastructure.yml
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

# ---------------------------------------------------------------------------
# Dry-run: show plan and exit
# ---------------------------------------------------------------------------
if $DRY_RUN; then
  echo "=== DRY RUN: backup_all.sh ==="
  echo "Timestamp : ${TIMESTAMP}"
  echo "Target dir: ${BACKUP_DIR}"
  echo ""
  echo "Databases to back up:"
  for entry in "${DATABASES[@]}"; do
    IFS='|' read -r name host port db user _pass <<< "$entry"
    printf "  %-16s %s@%s:%s/%s\n" "${name}" "${user}" "${host}" "${port}" "${db}"
  done
  echo ""
  echo "Files that would be created:"
  echo "  ${BACKUP_DIR}/<module>.sql.gz"
  echo "  ${BACKUP_DIR}/manifest.json"
  echo "  ${BACKUP_DIR}/backup.log"
  echo "=== DRY RUN complete. No changes made. ==="
  exit 0
fi

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------
mkdir -p "${BACKUP_DIR}"
LOG_FILE="${BACKUP_DIR}/backup.log"

log() { echo "[$(date +%H:%M:%S)] $*" | tee -a "${LOG_FILE}"; }

log "Starting backup — timestamp: ${TIMESTAMP}"
log "Target: ${BACKUP_DIR}"

MANIFEST_ENTRIES=""
BACKUP_ERRORS=0

# ---------------------------------------------------------------------------
# Helper: collect row counts for all user tables in a database
# Returns JSON array: [{"table":"foo","count":42},...]
# ---------------------------------------------------------------------------
table_counts_json() {
  local host="$1" port="$2" db="$3" user="$4" pass="$5"
  PGPASSWORD="$pass" psql -h "$host" -p "$port" -U "$user" -d "$db" -t -A -F'|' <<'SQL' 2>/dev/null | \
    awk -F'|' 'NF==2 && $1!="" {
      count++
      if (count>1) printf ","
      printf "{\"table\":\"%s\",\"count\":%s}", $1, $2
    } END { printf "" }' | { echo -n "["; cat; echo -n "]"; }
SELECT
  t.table_name,
  (xpath('/row/c/text()',
         query_to_xml(format('SELECT COUNT(*) AS c FROM %I.%I',
                             t.table_schema, t.table_name), false, true, '')))[1]::text::bigint
FROM information_schema.tables t
WHERE t.table_schema = 'public'
  AND t.table_type   = 'BASE TABLE'
ORDER BY t.table_name;
SQL
}

# ---------------------------------------------------------------------------
# Backup loop
# ---------------------------------------------------------------------------
for entry in "${DATABASES[@]}"; do
  IFS='|' read -r name host port db user pass <<< "$entry"
  DUMP_FILE="${BACKUP_DIR}/${name}.sql.gz"

  # Check reachability before dumping
  if ! PGPASSWORD="$pass" pg_isready -h "$host" -p "$port" -U "$user" -d "$db" -q 2>/dev/null; then
    log "WARN  ${name}: database not reachable at ${host}:${port} — skipping"
    BACKUP_ERRORS=$((BACKUP_ERRORS + 1))
    continue
  fi

  log "INFO  ${name}: dumping ${db} → ${name}.sql.gz"
  if PGPASSWORD="$pass" pg_dump \
      -h "$host" -p "$port" -U "$user" \
      --no-password \
      --format=plain \
      --no-owner \
      --no-acl \
      "$db" 2>>"${LOG_FILE}" | gzip -9 > "${DUMP_FILE}"; then

    SIZE=$(wc -c < "${DUMP_FILE}" | tr -d ' ')
    log "INFO  ${name}: done (${SIZE} bytes compressed)"

    # Collect table row counts for manifest
    COUNTS_JSON="$(table_counts_json "$host" "$port" "$db" "$user" "$pass")"
    MANIFEST_ENTRIES="${MANIFEST_ENTRIES}${MANIFEST_ENTRIES:+,}
    {
      \"name\": \"${name}\",
      \"database\": \"${db}\",
      \"host\": \"${host}\",
      \"port\": ${port},
      \"file\": \"${name}.sql.gz\",
      \"size_bytes\": ${SIZE},
      \"tables\": ${COUNTS_JSON}
    }"
  else
    log "ERROR ${name}: pg_dump failed"
    BACKUP_ERRORS=$((BACKUP_ERRORS + 1))
    rm -f "${DUMP_FILE}"
  fi
done

# ---------------------------------------------------------------------------
# Write manifest.json
# ---------------------------------------------------------------------------
MANIFEST_FILE="${BACKUP_DIR}/manifest.json"
cat > "${MANIFEST_FILE}" <<EOF
{
  "version": 1,
  "timestamp": "${TIMESTAMP}",
  "hostname": "$(hostname)",
  "backup_dir": "${BACKUP_DIR}",
  "errors": ${BACKUP_ERRORS},
  "databases": [${MANIFEST_ENTRIES}
  ]
}
EOF

log "INFO  Manifest written: manifest.json"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
TOTAL="${#DATABASES[@]}"
SUCCESS=$((TOTAL - BACKUP_ERRORS))
log "INFO  Backup complete: ${SUCCESS}/${TOTAL} databases backed up"

if [[ $BACKUP_ERRORS -gt 0 ]]; then
  log "WARN  ${BACKUP_ERRORS} database(s) were skipped or failed (see log)"
  exit 1
fi

# ---------------------------------------------------------------------------
# Prometheus textfile metric
# Written to node_exporter's textfile directory so Prometheus can scrape it.
# Metric: platform_backup_last_success_seconds{module} — Unix timestamp of
# the most recent successful backup for each module.
# Alert rule derives age: time() - platform_backup_last_success_seconds > 93600 (26h)
# ---------------------------------------------------------------------------
TEXTFILE_DIR="${PROMETHEUS_TEXTFILE_DIR:-/var/lib/prometheus/textfiles}"
METRIC_FILE="${TEXTFILE_DIR}/backup.prom"
NOW_EPOCH="$(date +%s)"

if [[ -d "$TEXTFILE_DIR" ]] || mkdir -p "$TEXTFILE_DIR" 2>/dev/null; then
  {
    echo "# HELP platform_backup_last_success_seconds Unix timestamp of the most recent successful pg_dump per module"
    echo "# TYPE platform_backup_last_success_seconds gauge"
    for entry in "${DATABASES[@]}"; do
      IFS='|' read -r name _host _port _db _user _pass <<< "$entry"
      if [[ -f "${BACKUP_DIR}/${name}.sql.gz" ]]; then
        echo "platform_backup_last_success_seconds{module=\"${name}\"} ${NOW_EPOCH}"
      fi
    done
  } > "${METRIC_FILE}.$$" && mv "${METRIC_FILE}.$$" "${METRIC_FILE}"
  log "INFO  Prometheus metric written: ${METRIC_FILE}"
else
  log "WARN  Prometheus textfile dir not writable: ${TEXTFILE_DIR} (set PROMETHEUS_TEXTFILE_DIR to override)"
fi

echo ""
echo "Backup directory: ${BACKUP_DIR}"
echo "Manifest:         ${MANIFEST_FILE}"
