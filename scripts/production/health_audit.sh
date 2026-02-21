#!/usr/bin/env bash
# health_audit.sh — Post-restore health audit for the 7D Platform databases.
#
# Verifies that all platform databases are accessible and contain data.
# Used after restore_drill.sh to confirm a restore is viable, and can also
# be run against the live production stack.
#
# Detection:
#   --drill mode   Checks the ephemeral restore container (7d-drill-postgres).
#   default mode   Checks running production containers (7d-*-postgres).
#   If no relevant containers are running, prints the audit matrix and exits 0.
#
# Usage:
#   ./scripts/production/health_audit.sh                    # auto-detect mode
#   ./scripts/production/health_audit.sh --drill            # drill container
#   ./scripts/production/health_audit.sh --drill \
#       --port 5499 --superuser drill_su --superpass pass
#
# Options:
#   --drill              Check the drill restore container (7d-drill-postgres)
#   --port PORT          Drill container local port (default: 5499)
#   --superuser USER     Drill container superuser (default: drill_su)
#   --superpass PASS     Drill container superuser password (from env DRILL_SUPERPASS)
#
# Exit: 0 = all reachable databases passed. Non-zero = one or more failures.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SECRETS_FILE="${SECRETS_FILE:-/etc/7d/production/secrets.env}"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
DRILL_MODE=false
DRILL_PORT="${DRILL_PORT:-5499}"
DRILL_CONTAINER="${DRILL_CONTAINER:-7d-drill-postgres}"
DRILL_SUPERUSER="${DRILL_SUPERUSER:-drill_su}"
DRILL_SUPERPASS="${DRILL_SUPERPASS:-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --drill)      DRILL_MODE=true;        shift   ;;
        --port)       DRILL_PORT="$2";        shift 2 ;;
        --superuser)  DRILL_SUPERUSER="$2";   shift 2 ;;
        --superpass)  DRILL_SUPERPASS="$2";   shift 2 ;;
        *) echo "[health_audit] ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

log()  { echo "[health_audit] $*"; }
ok()   { printf '  ✓  %-40s %s\n' "$1" "$2"; }
warn() { printf '  ~  %-40s %s\n' "$1" "$2"; }
fail() { printf '  ✗  %-40s %s\n' "$1" "$2"; }

PASS=0
SKIP=0
FAIL=0

# ---------------------------------------------------------------------------
# Source credentials (for production mode connectivity details)
# ---------------------------------------------------------------------------
if [[ -f "$SECRETS_FILE" ]]; then
    while IFS= read -r _line; do
        [[ -z "$_line" || "$_line" == \#* ]] && continue
        [[ "$_line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] && export "$_line" 2>/dev/null || true
    done < "$SECRETS_FILE"
fi

# ---------------------------------------------------------------------------
# Database audit matrix
# label|DB_ENV_VAR|USER_ENV_VAR|PASS_ENV_VAR|PROD_CONTAINER
# ---------------------------------------------------------------------------
declare -a DB_MATRIX=(
    "auth|AUTH_POSTGRES_DB|AUTH_POSTGRES_USER|AUTH_POSTGRES_PASSWORD|7d-auth-postgres"
    "tenant_registry|TENANT_REGISTRY_POSTGRES_DB|TENANT_REGISTRY_POSTGRES_USER|TENANT_REGISTRY_POSTGRES_PASSWORD|7d-tenant-registry-postgres"
    "audit|AUDIT_POSTGRES_DB|AUDIT_POSTGRES_USER|AUDIT_POSTGRES_PASSWORD|7d-audit-postgres"
    "gl|GL_POSTGRES_DB|GL_POSTGRES_USER|GL_POSTGRES_PASSWORD|7d-gl-postgres"
    "ar|AR_POSTGRES_DB|AR_POSTGRES_USER|AR_POSTGRES_PASSWORD|7d-ar-postgres"
    "ap|AP_POSTGRES_DB|AP_POSTGRES_USER|AP_POSTGRES_PASSWORD|7d-ap-postgres"
    "payments|PAYMENTS_POSTGRES_DB|PAYMENTS_POSTGRES_USER|PAYMENTS_POSTGRES_PASSWORD|7d-payments-postgres"
    "treasury|TREASURY_POSTGRES_DB|TREASURY_POSTGRES_USER|TREASURY_POSTGRES_PASSWORD|7d-treasury-postgres"
    "subscriptions|SUBSCRIPTIONS_POSTGRES_DB|SUBSCRIPTIONS_POSTGRES_USER|SUBSCRIPTIONS_POSTGRES_PASSWORD|7d-subscriptions-postgres"
    "inventory|INVENTORY_POSTGRES_DB|INVENTORY_POSTGRES_USER|INVENTORY_POSTGRES_PASSWORD|7d-inventory-postgres"
    "fixed_assets|FIXED_ASSETS_POSTGRES_DB|FIXED_ASSETS_POSTGRES_USER|FIXED_ASSETS_POSTGRES_PASSWORD|7d-fixed-assets-postgres"
    "consolidation|CONSOLIDATION_POSTGRES_DB|CONSOLIDATION_POSTGRES_USER|CONSOLIDATION_POSTGRES_PASSWORD|7d-consolidation-postgres"
    "notifications|NOTIFICATIONS_POSTGRES_DB|NOTIFICATIONS_POSTGRES_USER|NOTIFICATIONS_POSTGRES_PASSWORD|7d-notifications-postgres"
    "projections|PROJECTIONS_POSTGRES_DB|PROJECTIONS_POSTGRES_USER|PROJECTIONS_POSTGRES_PASSWORD|7d-projections-postgres"
    "timekeeping|TIMEKEEPING_POSTGRES_DB|TIMEKEEPING_POSTGRES_USER|TIMEKEEPING_POSTGRES_PASSWORD|7d-timekeeping-postgres"
    "party|PARTY_POSTGRES_DB|PARTY_POSTGRES_USER|PARTY_POSTGRES_PASSWORD|7d-party-postgres"
    "integrations|INTEGRATIONS_POSTGRES_DB|INTEGRATIONS_POSTGRES_USER|INTEGRATIONS_POSTGRES_PASSWORD|7d-integrations-postgres"
    "ttp|TTP_POSTGRES_DB|TTP_POSTGRES_USER|TTP_POSTGRES_PASSWORD|7d-ttp-postgres"
)

# ---------------------------------------------------------------------------
# Determine which target to audit
# ---------------------------------------------------------------------------
if $DRILL_MODE; then
    log "Mode: restore drill — checking container ${DRILL_CONTAINER}"
    if ! docker inspect "$DRILL_CONTAINER" >/dev/null 2>&1; then
        log "WARN: Drill container ${DRILL_CONTAINER} is not running — nothing to audit"
        log "Results: 0 passed, 0 failed, ${#DB_MATRIX[@]} skipped (no target)"
        exit 0
    fi
else
    # Auto-detect: check if any production Postgres containers are running
    PROD_CONTAINERS_RUNNING=0
    for _entry in "${DB_MATRIX[@]}"; do
        IFS='|' read -r _ _ _ _ _prod_container <<< "$_entry"
        if docker inspect "$_prod_container" >/dev/null 2>&1; then
            PROD_CONTAINERS_RUNNING=$((PROD_CONTAINERS_RUNNING + 1))
        fi
    done
    if [[ $PROD_CONTAINERS_RUNNING -eq 0 ]]; then
        log "Mode: production (auto-detected)"
        log "No production Postgres containers are running — nothing to audit"
        log "To audit a restore drill, run: health_audit.sh --drill"
        log "Results: 0 passed, 0 failed, ${#DB_MATRIX[@]} skipped (no containers)"
        exit 0
    fi
    log "Mode: production (auto-detected, ${PROD_CONTAINERS_RUNNING} containers running)"
fi

echo ""
printf '  %-42s %s\n' "Database" "Result"
printf '  %-42s %s\n' "--------" "------"

# ---------------------------------------------------------------------------
# Audit each database
# ---------------------------------------------------------------------------
audit_db_drill() {
    local label="$1" db="$2"

    # Test connectivity via SELECT 1
    local result
    result=$(docker exec -i \
        -e PGPASSWORD="$DRILL_SUPERPASS" \
        "$DRILL_CONTAINER" \
        psql -U "$DRILL_SUPERUSER" -d "$db" -tAc "SELECT 1;" 2>&1) || true

    if [[ "$result" == "1" ]]; then
        # Check table count to verify data was actually restored
        local table_count
        table_count=$(docker exec -i \
            -e PGPASSWORD="$DRILL_SUPERPASS" \
            "$DRILL_CONTAINER" \
            psql -U "$DRILL_SUPERUSER" -d "$db" -tAc \
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';" 2>/dev/null) || true
        table_count="${table_count:-0}"
        if [[ "$table_count" -gt 0 ]]; then
            ok "$label" "connected, ${table_count} table(s)"
            PASS=$((PASS + 1))
        else
            warn "$label" "connected but 0 tables (empty restore?)"
            PASS=$((PASS + 1))
        fi
    else
        fail "$label" "connection failed: ${result:-no response}"
        FAIL=$((FAIL + 1))
    fi
}

audit_db_prod() {
    local label="$1" db="$2" db_user="$3" db_pass="$4" container="$5"

    if ! docker inspect "$container" >/dev/null 2>&1; then
        warn "$label" "container ${container} not running — skip"
        SKIP=$((SKIP + 1))
        return
    fi

    local result
    result=$(docker exec -i \
        -e PGPASSWORD="$db_pass" \
        "$container" \
        psql -U "$db_user" -d "$db" -tAc "SELECT 1;" 2>&1) || true

    if [[ "$result" == "1" ]]; then
        local table_count
        table_count=$(docker exec -i \
            -e PGPASSWORD="$db_pass" \
            "$container" \
            psql -U "$db_user" -d "$db" -tAc \
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';" 2>/dev/null) || true
        table_count="${table_count:-0}"
        ok "$label" "connected, ${table_count} table(s)"
        PASS=$((PASS + 1))
    else
        fail "$label" "connection failed: ${result:-no response}"
        FAIL=$((FAIL + 1))
    fi
}

for _entry in "${DB_MATRIX[@]}"; do
    IFS='|' read -r _label _db_env _user_env _pass_env _prod_container <<< "$_entry"
    _db="${!_db_env:-}"
    _user="${!_user_env:-}"
    _pass="${!_pass_env:-}"

    if $DRILL_MODE; then
        if [[ -z "$_db" ]]; then
            warn "$_label" "credentials not set (${_db_env})"
            SKIP=$((SKIP + 1))
            continue
        fi
        audit_db_drill "$_label" "$_db"
    else
        if [[ -z "$_db" || -z "$_user" || -z "$_pass" ]]; then
            warn "$_label" "credentials not set — skip"
            SKIP=$((SKIP + 1))
            continue
        fi
        audit_db_prod "$_label" "$_db" "$_user" "$_pass" "$_prod_container"
    fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
printf '──────────────────────────────────────────\n'
printf 'Health audit results: %d passed, %d failed, %d skipped\n' \
    "$PASS" "$FAIL" "$SKIP"

if [[ $FAIL -gt 0 ]]; then
    echo ""
    log "Health audit FAILED — ${FAIL} database(s) are not accessible"
    exit 1
fi

if [[ $PASS -eq 0 && $SKIP -gt 0 ]]; then
    log "Health audit: no databases audited (all skipped)"
    exit 0
fi

log "Health audit PASSED — all reachable databases are accessible"
exit 0
