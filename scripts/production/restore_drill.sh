#!/usr/bin/env bash
# restore_drill.sh — Prove backup restore into a clean isolated target.
#
# Provisions an ephemeral Docker Postgres container as the "clean restore target",
# restores every database dump from a backup directory into it, then runs
# health_audit.sh to confirm the restored state is viable.
#
# Usage:
#   bash scripts/production/restore_drill.sh
#   bash scripts/production/restore_drill.sh --backup-dir /var/backups/7d-platform/2026-02-21_02-00-00
#   bash scripts/production/restore_drill.sh --dry-run
#   bash scripts/production/restore_drill.sh --no-cleanup   # leave container for inspection
#
# Options:
#   --backup-dir DIR   Specific backup directory to restore (default: latest in BACKUP_DIR)
#   --dry-run          Print steps without executing (no containers started)
#   --no-cleanup       Leave drill containers running after completion
#
# Required (sourced from SECRETS_FILE or pre-exported in environment):
#   *_POSTGRES_DB, *_POSTGRES_USER, *_POSTGRES_PASSWORD — per-module credentials
#   See scripts/production/env.example for the full list.
#
# Optional environment:
#   SECRETS_FILE   Path to secrets file (default: /etc/7d/production/secrets.env)
#   BACKUP_DIR     Root local backup directory (default: /var/backups/7d-platform)
#   DRILL_PORT     Local port for the drill Postgres container (default: 5499)
#
# RPO/RTO context:
#   Target RTO: 4 hours for critical databases (GL, AR, AP, Payments, Treasury, Auth)
#   Target RTO: 8 hours for standard databases (all others)
#   This drill validates the restore procedure and measures actual restore time.
#
# Exit: 0 = drill passed. Non-zero = one or more failures.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
SECRETS_FILE="${SECRETS_FILE:-/etc/7d/production/secrets.env}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/7d-platform}"
DRILL_PORT="${DRILL_PORT:-5499}"
DRILL_CONTAINER="7d-drill-postgres"
DRILL_NETWORK="7d-drill-net"
DRILL_SUPERUSER="drill_su"
DRILL_SUPERPASS="drillpass_$(date +%s)"

DRY_RUN=false
NO_CLEANUP=false
OVERRIDE_BACKUP_DIR=""

log()    { echo "[restore_drill] $*"; }
ok()     { echo "[restore_drill] OK:  $*"; }
err()    { echo "[restore_drill] ERR: $*" >&2; }
fail()   { echo "[restore_drill] FAIL: $*" >&2; exit 1; }
elapsed(){ echo $(( $(date +%s) - $1 ))s; }

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --backup-dir) OVERRIDE_BACKUP_DIR="$2"; shift 2 ;;
        --dry-run)    DRY_RUN=true;              shift   ;;
        --no-cleanup) NO_CLEANUP=true;           shift   ;;
        *) fail "Unknown argument: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Source credentials
# ---------------------------------------------------------------------------
if [[ -f "$SECRETS_FILE" ]]; then
    while IFS= read -r _line; do
        [[ -z "$_line" || "$_line" == \#* ]] && continue
        [[ "$_line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] && export "$_line"
    done < "$SECRETS_FILE"
    log "Loaded secrets from $SECRETS_FILE"
else
    log "WARN: $SECRETS_FILE not found — expecting credentials in environment"
fi

# ---------------------------------------------------------------------------
# Database list: label|DB_ENV_VAR|USER_ENV_VAR|PASS_ENV_VAR
# Restore order: platform first (auth, tenant-registry, audit), then financial,
# then remaining modules. Mirrors DR runbook restore sequence.
# ---------------------------------------------------------------------------
declare -a DB_CONFIGS=(
    "auth|AUTH_POSTGRES_DB|AUTH_POSTGRES_USER|AUTH_POSTGRES_PASSWORD"
    "tenant_registry|TENANT_REGISTRY_POSTGRES_DB|TENANT_REGISTRY_POSTGRES_USER|TENANT_REGISTRY_POSTGRES_PASSWORD"
    "audit|AUDIT_POSTGRES_DB|AUDIT_POSTGRES_USER|AUDIT_POSTGRES_PASSWORD"
    "gl|GL_POSTGRES_DB|GL_POSTGRES_USER|GL_POSTGRES_PASSWORD"
    "ar|AR_POSTGRES_DB|AR_POSTGRES_USER|AR_POSTGRES_PASSWORD"
    "ap|AP_POSTGRES_DB|AP_POSTGRES_USER|AP_POSTGRES_PASSWORD"
    "payments|PAYMENTS_POSTGRES_DB|PAYMENTS_POSTGRES_USER|PAYMENTS_POSTGRES_PASSWORD"
    "treasury|TREASURY_POSTGRES_DB|TREASURY_POSTGRES_USER|TREASURY_POSTGRES_PASSWORD"
    "subscriptions|SUBSCRIPTIONS_POSTGRES_DB|SUBSCRIPTIONS_POSTGRES_USER|SUBSCRIPTIONS_POSTGRES_PASSWORD"
    "inventory|INVENTORY_POSTGRES_DB|INVENTORY_POSTGRES_USER|INVENTORY_POSTGRES_PASSWORD"
    "fixed_assets|FIXED_ASSETS_POSTGRES_DB|FIXED_ASSETS_POSTGRES_USER|FIXED_ASSETS_POSTGRES_PASSWORD"
    "consolidation|CONSOLIDATION_POSTGRES_DB|CONSOLIDATION_POSTGRES_USER|CONSOLIDATION_POSTGRES_PASSWORD"
    "notifications|NOTIFICATIONS_POSTGRES_DB|NOTIFICATIONS_POSTGRES_USER|NOTIFICATIONS_POSTGRES_PASSWORD"
    "projections|PROJECTIONS_POSTGRES_DB|PROJECTIONS_POSTGRES_USER|PROJECTIONS_POSTGRES_PASSWORD"
    "timekeeping|TIMEKEEPING_POSTGRES_DB|TIMEKEEPING_POSTGRES_USER|TIMEKEEPING_POSTGRES_PASSWORD"
    "party|PARTY_POSTGRES_DB|PARTY_POSTGRES_USER|PARTY_POSTGRES_PASSWORD"
    "integrations|INTEGRATIONS_POSTGRES_DB|INTEGRATIONS_POSTGRES_USER|INTEGRATIONS_POSTGRES_PASSWORD"
    "ttp|TTP_POSTGRES_DB|TTP_POSTGRES_USER|TTP_POSTGRES_PASSWORD"
)

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
    fail "docker not found — required for restore drill"
fi

# ---------------------------------------------------------------------------
# Resolve backup directory
# ---------------------------------------------------------------------------
if [[ -n "$OVERRIDE_BACKUP_DIR" ]]; then
    TARGET_BACKUP="$OVERRIDE_BACKUP_DIR"
else
    TARGET_BACKUP="$(
        ls -1d "${BACKUP_DIR}"/????-??-??_??-??-?? 2>/dev/null \
        | sort -r | head -1 || true
    )"
fi

if [[ -z "$TARGET_BACKUP" || ! -d "$TARGET_BACKUP" ]]; then
    if $DRY_RUN; then
        TARGET_BACKUP="${BACKUP_DIR}/(none — dry-run)"
        BACKUP_TIMESTAMP="(no backup found — dry-run only)"
    else
        fail "No backup directory found (BACKUP_DIR=${BACKUP_DIR}). Run backup_all_dbs.sh first."
    fi
else
    BACKUP_TIMESTAMP="$(basename "$TARGET_BACKUP")"
fi
log "Restore drill starting"
log "Backup source:    $TARGET_BACKUP"
log "Drill container:  $DRILL_CONTAINER (port ${DRILL_PORT})"
DRILL_START=$(date +%s)

# ---------------------------------------------------------------------------
# Verify backup manifest checksums
# ---------------------------------------------------------------------------
MANIFEST="${TARGET_BACKUP}/MANIFEST.txt"
if [[ -f "$MANIFEST" ]]; then
    log "Verifying MANIFEST.txt checksums ..."
    CHECKSUM_FAILURES=0
    while IFS= read -r _mline; do
        [[ -z "$_mline" || "$_mline" == \#* ]] && continue
        _sha="${_mline%%  *}"
        _fname="${_mline##*  }"
        _fpath="${TARGET_BACKUP}/${_fname}"
        if [[ ! -f "$_fpath" ]]; then
            err "Missing file: $_fname"
            CHECKSUM_FAILURES=$((CHECKSUM_FAILURES + 1))
            continue
        fi
        _actual="$(sha256sum "$_fpath" | awk '{print $1}')"
        if [[ "$_actual" != "$_sha" ]]; then
            err "Checksum mismatch: $_fname (expected $_sha, got $_actual)"
            CHECKSUM_FAILURES=$((CHECKSUM_FAILURES + 1))
        fi
    done < <(grep -v '^#' "$MANIFEST" | grep -v '^$' || true)
    if [[ $CHECKSUM_FAILURES -gt 0 ]]; then
        fail "Backup integrity check failed — ${CHECKSUM_FAILURES} file(s) corrupted or missing"
    fi
    ok "Manifest checksums verified"
else
    log "WARN: No MANIFEST.txt — skipping checksum verification"
fi

# ---------------------------------------------------------------------------
# Cleanup function
# ---------------------------------------------------------------------------
drill_cleanup() {
    if $NO_CLEANUP; then
        log "Skipping cleanup (--no-cleanup): container ${DRILL_CONTAINER} left running"
        log "  Stop manually: docker rm -f ${DRILL_CONTAINER} && docker network rm ${DRILL_NETWORK}"
        return
    fi
    log "Cleaning up drill container and network ..."
    docker rm -f "$DRILL_CONTAINER" >/dev/null 2>&1 || true
    docker network rm "$DRILL_NETWORK" >/dev/null 2>&1 || true
    log "Cleanup complete"
}
trap drill_cleanup EXIT

# ---------------------------------------------------------------------------
# Dry-run gate
# ---------------------------------------------------------------------------
if $DRY_RUN; then
    log "DRY-RUN mode — printing plan without executing"
    log ""
    log "Would create Docker network:  $DRILL_NETWORK"
    log "Would start container:        $DRILL_CONTAINER (postgres:16) on port ${DRILL_PORT}"
    log "Would restore ${#DB_CONFIGS[@]} databases from: $TARGET_BACKUP"
    for _config in "${DB_CONFIGS[@]}"; do
        IFS='|' read -r _label _db_env _user_env _pass_env <<< "$_config"
        _db="${!_db_env:-<${_db_env} not set>}"
        log "  - $_label: ${_db}.sql.gz"
    done
    log "Would run: scripts/production/health_audit.sh --drill"
    log ""
    log "DRY-RUN complete — no containers were started"
    exit 0
fi

# ---------------------------------------------------------------------------
# Start restore target container
# ---------------------------------------------------------------------------
log "Creating isolated network: $DRILL_NETWORK"
docker network create "$DRILL_NETWORK" >/dev/null 2>&1 || true

log "Starting clean Postgres restore target: $DRILL_CONTAINER"
docker rm -f "$DRILL_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
    --name "$DRILL_CONTAINER" \
    --network "$DRILL_NETWORK" \
    -p "${DRILL_PORT}:5432" \
    -e POSTGRES_USER="$DRILL_SUPERUSER" \
    -e POSTGRES_PASSWORD="$DRILL_SUPERPASS" \
    -e POSTGRES_DB=postgres \
    postgres:16 \
    >/dev/null

log "Waiting for restore target to accept connections ..."
READY_TRIES=0
until docker exec "$DRILL_CONTAINER" \
    pg_isready -U "$DRILL_SUPERUSER" -d postgres -q 2>/dev/null; do
    READY_TRIES=$((READY_TRIES + 1))
    if [[ $READY_TRIES -gt 30 ]]; then
        fail "Restore target did not become ready within 30s"
    fi
    sleep 1
done
ok "Restore target ready (${READY_TRIES}s)"

# ---------------------------------------------------------------------------
# Restore databases
# ---------------------------------------------------------------------------
RESTORE_FAILURES=0
declare -a RESTORE_REPORT=()

psql_drill() {
    docker exec -i -e PGPASSWORD="$DRILL_SUPERPASS" "$DRILL_CONTAINER" \
        psql -U "$DRILL_SUPERUSER" -d postgres -q "$@"
}

restore_db() {
    local label="$1" db="$2" db_user="$3" db_pass="$4"
    local dump_file="${TARGET_BACKUP}/${db}.sql.gz"
    local t_start
    t_start=$(date +%s)

    if [[ ! -f "$dump_file" ]]; then
        err "Dump file not found: ${dump_file} — skipping ${label}"
        RESTORE_REPORT+=("SKIP  ${label}: dump file missing")
        return 1
    fi

    log "  Restoring ${label} (${db}) ..."

    # Create role and database on the drill target
    psql_drill <<SQL 2>/dev/null || true
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${db_user}') THEN
    CREATE ROLE "${db_user}" LOGIN PASSWORD '${db_pass}';
  END IF;
END
\$\$;
SQL
    psql_drill -c "DROP DATABASE IF EXISTS \"${db}\";" 2>/dev/null || true
    psql_drill -c "CREATE DATABASE \"${db}\" OWNER \"${db_user}\";" 2>/dev/null

    # Restore the dump
    local rc=0
    if gunzip -c "$dump_file" | docker exec -i \
        -e PGPASSWORD="$DRILL_SUPERPASS" "$DRILL_CONTAINER" \
        psql -U "$DRILL_SUPERUSER" -d "$db" -q 2>/dev/null; then
        local elapsed
        elapsed=$(( $(date +%s) - t_start ))
        ok "  ${label}: restored in ${elapsed}s"
        RESTORE_REPORT+=("PASS  ${label} (${db}): ${elapsed}s")
    else
        err "  ${label}: restore FAILED"
        RESTORE_REPORT+=("FAIL  ${label} (${db}): psql error")
        rc=1
    fi
    return $rc
}

log ""
log "Restoring ${#DB_CONFIGS[@]} databases (restore order: platform → financial → modules) ..."
for _config in "${DB_CONFIGS[@]}"; do
    IFS='|' read -r _label _db_env _user_env _pass_env <<< "$_config"
    _db="${!_db_env:-}"
    _user="${!_user_env:-}"
    _pass="${!_pass_env:-}"
    if [[ -z "$_db" || -z "$_user" || -z "$_pass" ]]; then
        err "Missing credentials for ${_label} — check ${_db_env}, ${_user_env}, ${_pass_env}"
        RESTORE_REPORT+=("SKIP  ${_label}: credentials not set")
        RESTORE_FAILURES=$((RESTORE_FAILURES + 1))
        continue
    fi
    if ! restore_db "$_label" "$_db" "$_user" "$_pass"; then
        RESTORE_FAILURES=$((RESTORE_FAILURES + 1))
    fi
done

# ---------------------------------------------------------------------------
# Run health audit against the drill target
# ---------------------------------------------------------------------------
log ""
log "Running health_audit.sh against restore target ..."
export DRILL_PORT DRILL_CONTAINER DRILL_SUPERUSER DRILL_SUPERPASS
AUDIT_RESULT=0
bash "${SCRIPT_DIR}/health_audit.sh" --drill \
    --port "$DRILL_PORT" \
    --superuser "$DRILL_SUPERUSER" \
    --superpass "$DRILL_SUPERPASS" \
    || AUDIT_RESULT=$?

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
DRILL_ELAPSED=$(elapsed "$DRILL_START")
log ""
log "============================="
log "Restore drill summary"
log "============================="
log "Backup source:   $BACKUP_TIMESTAMP"
log "Total duration:  $DRILL_ELAPSED"
log ""
log "Database restore results:"
for _line in "${RESTORE_REPORT[@]}"; do
    log "  $_line"
done
log ""

if [[ $RESTORE_FAILURES -gt 0 ]]; then
    err "${RESTORE_FAILURES} database(s) failed to restore"
fi
if [[ $AUDIT_RESULT -ne 0 ]]; then
    err "health_audit.sh reported failures"
fi

if [[ $RESTORE_FAILURES -eq 0 && $AUDIT_RESULT -eq 0 ]]; then
    log "Restore drill PASSED — all databases restored and audit clean"
    log "Elapsed: $DRILL_ELAPSED"
    exit 0
else
    err "Restore drill FAILED — see errors above"
    exit 1
fi
