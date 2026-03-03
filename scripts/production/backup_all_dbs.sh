#!/usr/bin/env bash
# backup_all_dbs.sh — Dump all 25 7D Platform Postgres databases to local backup storage.
#
# Runs pg_dump (per-database) and pg_dumpall --globals-only via docker exec.
# Each invocation creates a timestamped directory under BACKUP_DIR containing
# one .sql.gz per database plus a globals dump and a SHA-256 manifest.
#
# Usage:
#   bash scripts/production/backup_all_dbs.sh
#   bash scripts/production/backup_all_dbs.sh --dry-run
#
# Required environment (sourced automatically from SECRETS_FILE):
#   *_POSTGRES_DB, *_POSTGRES_USER, *_POSTGRES_PASSWORD  — per-service credentials
#   See scripts/production/env.example for the full list.
#
# Optional environment:
#   SECRETS_FILE      Path to production secrets (default: /etc/7d/production/secrets.env)
#   BACKUP_DIR        Root directory for local backups (default: /var/backups/7d-platform)
#
# Exit: 0 = all databases backed up successfully. Non-zero = one or more failures.

set -euo pipefail

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
DRY_RUN=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=true; shift ;;
        *) echo "[backup_all_dbs] ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
SECRETS_FILE="${SECRETS_FILE:-/etc/7d/production/secrets.env}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/7d-platform}"
TIMESTAMP="$(date -u +%Y-%m-%d_%H-%M-%S)"
BACKUP_RUN_DIR="${BACKUP_DIR}/${TIMESTAMP}"

log()  { echo "[backup_all_dbs] $*"; }
err()  { echo "[backup_all_dbs] ERROR: $*" >&2; }
fail() { echo "[backup_all_dbs] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Source credentials from secrets file
# ---------------------------------------------------------------------------
if [[ -f "$SECRETS_FILE" ]]; then
    while IFS= read -r _line; do
        [[ -z "$_line" || "$_line" == \#* ]] && continue
        if [[ "$_line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
            export "$_line"
        fi
    done < "$SECRETS_FILE"
    log "Loaded secrets from $SECRETS_FILE"
else
    log "WARN: $SECRETS_FILE not found — expecting credentials already in environment"
fi

# ---------------------------------------------------------------------------
# Database list: "container|DB_ENV_VAR|USER_ENV_VAR|PASS_ENV_VAR"
# Mirrors the containers defined in docker-compose.data.yml.
# ---------------------------------------------------------------------------
declare -a DB_CONFIGS=(
    "7d-auth-postgres|AUTH_POSTGRES_DB|AUTH_POSTGRES_USER|AUTH_POSTGRES_PASSWORD"
    "7d-ar-postgres|AR_POSTGRES_DB|AR_POSTGRES_USER|AR_POSTGRES_PASSWORD"
    "7d-subscriptions-postgres|SUBSCRIPTIONS_POSTGRES_DB|SUBSCRIPTIONS_POSTGRES_USER|SUBSCRIPTIONS_POSTGRES_PASSWORD"
    "7d-payments-postgres|PAYMENTS_POSTGRES_DB|PAYMENTS_POSTGRES_USER|PAYMENTS_POSTGRES_PASSWORD"
    "7d-notifications-postgres|NOTIFICATIONS_POSTGRES_DB|NOTIFICATIONS_POSTGRES_USER|NOTIFICATIONS_POSTGRES_PASSWORD"
    "7d-gl-postgres|GL_POSTGRES_DB|GL_POSTGRES_USER|GL_POSTGRES_PASSWORD"
    "7d-projections-postgres|PROJECTIONS_POSTGRES_DB|PROJECTIONS_POSTGRES_USER|PROJECTIONS_POSTGRES_PASSWORD"
    "7d-audit-postgres|AUDIT_POSTGRES_DB|AUDIT_POSTGRES_USER|AUDIT_POSTGRES_PASSWORD"
    "7d-tenant-registry-postgres|TENANT_REGISTRY_POSTGRES_DB|TENANT_REGISTRY_POSTGRES_USER|TENANT_REGISTRY_POSTGRES_PASSWORD"
    "7d-inventory-postgres|INVENTORY_POSTGRES_DB|INVENTORY_POSTGRES_USER|INVENTORY_POSTGRES_PASSWORD"
    "7d-ap-postgres|AP_POSTGRES_DB|AP_POSTGRES_USER|AP_POSTGRES_PASSWORD"
    "7d-treasury-postgres|TREASURY_POSTGRES_DB|TREASURY_POSTGRES_USER|TREASURY_POSTGRES_PASSWORD"
    "7d-fixed-assets-postgres|FIXED_ASSETS_POSTGRES_DB|FIXED_ASSETS_POSTGRES_USER|FIXED_ASSETS_POSTGRES_PASSWORD"
    "7d-consolidation-postgres|CONSOLIDATION_POSTGRES_DB|CONSOLIDATION_POSTGRES_USER|CONSOLIDATION_POSTGRES_PASSWORD"
    "7d-timekeeping-postgres|TIMEKEEPING_POSTGRES_DB|TIMEKEEPING_POSTGRES_USER|TIMEKEEPING_POSTGRES_PASSWORD"
    "7d-party-postgres|PARTY_POSTGRES_DB|PARTY_POSTGRES_USER|PARTY_POSTGRES_PASSWORD"
    "7d-integrations-postgres|INTEGRATIONS_POSTGRES_DB|INTEGRATIONS_POSTGRES_USER|INTEGRATIONS_POSTGRES_PASSWORD"
    "7d-ttp-postgres|TTP_POSTGRES_DB|TTP_POSTGRES_USER|TTP_POSTGRES_PASSWORD"
    "7d-maintenance-postgres|MAINTENANCE_POSTGRES_DB|MAINTENANCE_POSTGRES_USER|MAINTENANCE_POSTGRES_PASSWORD"
    "7d-pdf-editor-postgres|PDF_EDITOR_POSTGRES_DB|PDF_EDITOR_POSTGRES_USER|PDF_EDITOR_POSTGRES_PASSWORD"
    "7d-shipping-receiving-postgres|SHIPPING_RECEIVING_POSTGRES_DB|SHIPPING_RECEIVING_POSTGRES_USER|SHIPPING_RECEIVING_POSTGRES_PASSWORD"
    "7d-numbering-postgres|NUMBERING_POSTGRES_DB|NUMBERING_POSTGRES_USER|NUMBERING_POSTGRES_PASSWORD"
    "7d-doc-mgmt-postgres|DOC_MGMT_POSTGRES_DB|DOC_MGMT_POSTGRES_USER|DOC_MGMT_POSTGRES_PASSWORD"
    "7d-workflow-postgres|WORKFLOW_POSTGRES_DB|WORKFLOW_POSTGRES_USER|WORKFLOW_POSTGRES_PASSWORD"
    "7d-workforce-competence-postgres|WC_POSTGRES_DB|WC_POSTGRES_USER|WC_POSTGRES_PASSWORD"
)

# ---------------------------------------------------------------------------
# Dry-run mode: list all configured databases and exit
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "true" ]]; then
    log "DRY RUN — listing ${#DB_CONFIGS[@]} configured databases:"
    for _config in "${DB_CONFIGS[@]}"; do
        IFS='|' read -r _container _db_env _user_env _pass_env <<< "$_config"
        log "  ${_container}  (${_db_env})"
    done
    log ""
    log "Total: ${#DB_CONFIGS[@]} databases configured for backup."
    exit 0
fi

# ---------------------------------------------------------------------------
# Preflight checks
# ---------------------------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
    fail "docker not found in PATH — is Docker installed?"
fi

mkdir -p "$BACKUP_RUN_DIR" || fail "Cannot create backup directory: $BACKUP_RUN_DIR"
log "Backup run directory: $BACKUP_RUN_DIR"

FAILURES=0

# ---------------------------------------------------------------------------
# Helper: dump one database
# ---------------------------------------------------------------------------
dump_db() {
    local container="$1"
    local db="$2"
    local user="$3"
    local pass="$4"
    local out_file="${BACKUP_RUN_DIR}/${db}.sql.gz"

    if ! docker inspect "$container" >/dev/null 2>&1; then
        err "Container not running: ${container}"
        return 1
    fi

    log "  Dumping ${db} from ${container}"
    if docker exec -i -e PGPASSWORD="$pass" "$container" \
        pg_dump -U "$user" -d "$db" --no-password \
        | gzip -6 > "$out_file"; then
        log "    OK: $(du -sh "$out_file" | cut -f1)"
        return 0
    else
        err "pg_dump failed for database: ${db} (container: ${container})"
        rm -f "$out_file"
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Dump each database
# ---------------------------------------------------------------------------
for _config in "${DB_CONFIGS[@]}"; do
    IFS='|' read -r _container _db_env _user_env _pass_env <<< "$_config"

    _db="${!_db_env:-}"
    _user="${!_user_env:-}"
    _pass="${!_pass_env:-}"

    if [[ -z "$_db" || -z "$_user" || -z "$_pass" ]]; then
        err "Missing credentials for container ${_container} — check ${_db_env}, ${_user_env}, ${_pass_env}"
        FAILURES=$((FAILURES + 1))
        continue
    fi

    if ! dump_db "$_container" "$_db" "$_user" "$_pass"; then
        FAILURES=$((FAILURES + 1))
    fi
done

# ---------------------------------------------------------------------------
# Dump globals (roles) from auth container — best effort
# ---------------------------------------------------------------------------
_globals_container="7d-auth-postgres"
_globals_user="${AUTH_POSTGRES_USER:-}"
_globals_pass="${AUTH_POSTGRES_PASSWORD:-}"
_globals_file="${BACKUP_RUN_DIR}/globals.sql.gz"

log "  Dumping globals (roles) from ${_globals_container}"
if [[ -n "$_globals_pass" ]] && docker inspect "$_globals_container" >/dev/null 2>&1; then
    if docker exec -i -e PGPASSWORD="$_globals_pass" "$_globals_container" \
        pg_dumpall -U "$_globals_user" --globals-only \
        | gzip -6 > "$_globals_file"; then
        log "    OK: $(du -sh "$_globals_file" | cut -f1)"
    else
        err "pg_dumpall --globals-only failed"
        rm -f "$_globals_file"
        FAILURES=$((FAILURES + 1))
    fi
else
    log "    WARN: Skipping globals — container unavailable or no credentials"
fi

# ---------------------------------------------------------------------------
# Write SHA-256 manifest
# ---------------------------------------------------------------------------
_manifest="${BACKUP_RUN_DIR}/MANIFEST.txt"
{
    echo "# 7D Platform backup manifest"
    echo "# Timestamp: ${TIMESTAMP}"
    echo "# Host: $(hostname -f 2>/dev/null || hostname)"
    echo "# Format: sha256  filename"
    echo ""
    for _f in "${BACKUP_RUN_DIR}"/*.sql.gz; do
        [[ -f "$_f" ]] || continue
        sha256sum "$_f" | awk -v b="$(basename "$_f")" '{print $1 "  " b}'
    done
} > "$_manifest"
log "Manifest written: $_manifest"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
_file_count=$(find "$BACKUP_RUN_DIR" -name "*.sql.gz" | wc -l)
_total_size=$(du -sh "$BACKUP_RUN_DIR" | cut -f1)
log ""
log "Backup complete: ${_file_count} dump file(s), ${_total_size} total"
log "Location: $BACKUP_RUN_DIR"

if [[ $FAILURES -gt 0 ]]; then
    err "${FAILURES} database(s) failed — backup run is INCOMPLETE."
    exit 1
fi

log "All databases backed up successfully."
exit 0
