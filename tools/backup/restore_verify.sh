#!/usr/bin/env bash
#
# Phase 16: Quarterly Restore Verification Test
#
# Purpose: Verify that backups are restorable and data integrity is maintained
# Frequency: Run quarterly (manual trigger via CI or cron)
# Scope: Restore one module DB to scratch DB and verify integrity
#
# Usage:
#   ./tools/backup/restore_verify.sh <backup_file> <module_name>
#
# Example:
#   ./tools/backup/restore_verify.sh backups/2026-02-16/ar_2026-02-16.sql.gz ar
#
# Exit Codes:
#   0 - Restore and verification successful
#   1 - Restore failed
#   2 - Integrity check failed
#   3 - Invalid arguments

set -euo pipefail

# Configuration
BACKUP_FILE="${1:-}"
MODULE_NAME="${2:-}"
SCRATCH_DB_NAME="restore_verify_scratch_$(date +%s)"
POSTGRES_HOST="${POSTGRES_HOST:-localhost}"
POSTGRES_PORT="${POSTGRES_PORT:-5432}"
POSTGRES_USER="${POSTGRES_USER:-postgres}"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Cleanup function
cleanup() {
    if [ -n "${SCRATCH_DB_NAME:-}" ]; then
        log_info "Cleaning up scratch database: ${SCRATCH_DB_NAME}"
        PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d postgres \
            -c "DROP DATABASE IF EXISTS ${SCRATCH_DB_NAME};" \
            2>/dev/null || true
    fi
}

# Set trap for cleanup on exit
trap cleanup EXIT

# Validate arguments
if [ -z "${BACKUP_FILE}" ] || [ -z "${MODULE_NAME}" ]; then
    log_error "Usage: $0 <backup_file> <module_name>"
    log_error "Example: $0 backups/2026-02-16/ar_2026-02-16.sql.gz ar"
    exit 3
fi

if [ ! -f "${BACKUP_FILE}" ]; then
    log_error "Backup file not found: ${BACKUP_FILE}"
    exit 3
fi

log_info "========================================="
log_info "Restore Verification Test"
log_info "Backup: ${BACKUP_FILE}"
log_info "Module: ${MODULE_NAME}"
log_info "Scratch DB: ${SCRATCH_DB_NAME}"
log_info "========================================="

# Step 1: Create scratch database
log_step "Creating scratch database..."
PGPASSWORD="${POSTGRES_PASSWORD}" psql \
    -h "${POSTGRES_HOST}" \
    -p "${POSTGRES_PORT}" \
    -U "${POSTGRES_USER}" \
    -d postgres \
    -c "CREATE DATABASE ${SCRATCH_DB_NAME};" || {
    log_error "Failed to create scratch database"
    exit 1
}
log_info "✓ Scratch database created"

# Step 2: Restore backup to scratch database
log_step "Restoring backup to scratch database..."
gunzip -c "${BACKUP_FILE}" | PGPASSWORD="${POSTGRES_PASSWORD}" psql \
    -h "${POSTGRES_HOST}" \
    -p "${POSTGRES_PORT}" \
    -U "${POSTGRES_USER}" \
    -d "${SCRATCH_DB_NAME}" \
    2>&1 | grep -v "^WARNING:" | grep -v "^NOTICE:" || {
    log_error "Failed to restore backup"
    exit 1
}
log_info "✓ Backup restored successfully"

# Step 3: Run module-specific integrity checks
log_step "Running integrity checks..."

case "${MODULE_NAME}" in
    ar)
        # AR Module Integrity Checks
        log_info "Running AR module integrity checks..."

        # Check 1: Invoice count
        invoice_count=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM invoices;" 2>/dev/null || echo "0")
        log_info "  - Invoices: ${invoice_count}"

        # Check 2: Verify no NULL required fields
        null_check=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM invoices WHERE app_id IS NULL OR amount_cents IS NULL;" 2>/dev/null || echo "-1")

        if [ "${null_check}" != "0" ] && [ "${null_check}" != " 0" ]; then
            log_error "  ✗ Found NULL values in required fields"
            exit 2
        fi
        log_info "  ✓ No NULL values in required fields"

        # Check 3: Verify outbox table exists
        outbox_exists=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM pg_tables WHERE tablename = 'events_outbox';" 2>/dev/null || echo "0")

        if [ "${outbox_exists}" != "1" ] && [ "${outbox_exists}" != " 1" ]; then
            log_error "  ✗ events_outbox table not found"
            exit 2
        fi
        log_info "  ✓ events_outbox table exists"
        ;;

    payments)
        # Payments Module Integrity Checks
        log_info "Running Payments module integrity checks..."

        # Check 1: Payment attempts count
        attempts_count=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM payment_attempts;" 2>/dev/null || echo "0")
        log_info "  - Payment attempts: ${attempts_count}"

        # Check 2: UNIQUE constraint on (app_id, payment_id, attempt_no)
        unique_violations=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM (SELECT app_id, payment_id, attempt_no, COUNT(*) as cnt FROM payment_attempts GROUP BY app_id, payment_id, attempt_no HAVING COUNT(*) > 1) AS duplicates;" 2>/dev/null || echo "-1")

        if [ "${unique_violations}" != "0" ] && [ "${unique_violations}" != " 0" ]; then
            log_error "  ✗ Found UNIQUE constraint violations"
            exit 2
        fi
        log_info "  ✓ No UNIQUE constraint violations"
        ;;

    subscriptions)
        # Subscriptions Module Integrity Checks
        log_info "Running Subscriptions module integrity checks..."

        # Check 1: Subscriptions count
        sub_count=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM subscriptions;" 2>/dev/null || echo "0")
        log_info "  - Subscriptions: ${sub_count}"

        # Check 2: Verify status enum values are valid
        invalid_status=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM subscriptions WHERE status::text NOT IN ('active', 'suspended', 'past_due');" 2>/dev/null || echo "-1")

        if [ "${invalid_status}" != "0" ] && [ "${invalid_status}" != " 0" ]; then
            log_error "  ✗ Found invalid status values"
            exit 2
        fi
        log_info "  ✓ All status values are valid"
        ;;

    gl)
        # GL Module Integrity Checks
        log_info "Running GL module integrity checks..."

        # Check 1: Journal entries count
        entries_count=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM journal_entries;" 2>/dev/null || echo "0")
        log_info "  - Journal entries: ${entries_count}"

        # Check 2: Verify GL balance (debits = credits)
        balance=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT SUM(debit_cents) - SUM(credit_cents) FROM journal_entries;" 2>/dev/null || echo "-999999")

        if [ "${balance}" != "0" ] && [ "${balance}" != " 0" ] && [ "${balance}" != "" ]; then
            log_error "  ✗ GL not balanced (debits != credits): ${balance}"
            exit 2
        fi
        log_info "  ✓ GL balanced (debits = credits)"
        ;;

    notifications)
        # Notifications Module Integrity Checks
        log_info "Running Notifications module integrity checks..."

        # Check 1: Verify tables exist
        tables_exist=$(PGPASSWORD="${POSTGRES_PASSWORD}" psql \
            -h "${POSTGRES_HOST}" \
            -p "${POSTGRES_PORT}" \
            -U "${POSTGRES_USER}" \
            -d "${SCRATCH_DB_NAME}" \
            -t -c "SELECT COUNT(*) FROM pg_tables WHERE schemaname = 'public';" 2>/dev/null || echo "0")

        if [ "${tables_exist}" == "0" ] || [ "${tables_exist}" == " 0" ]; then
            log_error "  ✗ No tables found in database"
            exit 2
        fi
        log_info "  ✓ Database tables exist (count: ${tables_exist})"
        ;;

    *)
        log_warn "No specific integrity checks defined for module: ${MODULE_NAME}"
        log_info "  ✓ Restore completed (generic verification only)"
        ;;
esac

# Step 4: Summary
log_info "========================================="
log_info "✅ Restore Verification PASSED"
log_info "Module: ${MODULE_NAME}"
log_info "Backup: ${BACKUP_FILE}"
log_info "All integrity checks passed"
log_info "========================================="

exit 0
