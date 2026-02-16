#!/usr/bin/env bash
#
# Phase 16: Daily Backup Script for All Module Databases
#
# Purpose: Create consistent backups of all module databases with timestamp
# Frequency: Run daily via cron
# Retention: External policy (recommend 30 days)
#
# Usage:
#   ./tools/backup/backup_all.sh [backup_dir]
#
# Environment Variables Required:
#   - AR_DATABASE_URL
#   - PAYMENTS_DATABASE_URL
#   - SUBSCRIPTIONS_DATABASE_URL
#   - GL_DATABASE_URL
#   - NOTIFICATIONS_DATABASE_URL (optional)

set -euo pipefail

# Configuration
BACKUP_DIR="${1:-./backups/$(date +%Y-%m-%d_%H-%M-%S)}"
TIMESTAMP=$(date +%Y-%m-%d_%H-%M-%S)
LOG_FILE="${BACKUP_DIR}/backup.log"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1" | tee -a "${LOG_FILE}"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1" | tee -a "${LOG_FILE}"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1" | tee -a "${LOG_FILE}"
}

# Create backup directory
mkdir -p "${BACKUP_DIR}"

log_info "Starting backup process at ${TIMESTAMP}"
log_info "Backup directory: ${BACKUP_DIR}"

# Function to backup a single database
backup_database() {
    local module_name=$1
    local db_url_var=$2
    local db_url="${!db_url_var}"

    if [ -z "${db_url}" ]; then
        log_warn "Skipping ${module_name}: ${db_url_var} not set"
        return 0
    fi

    log_info "Backing up ${module_name} database..."

    # Extract connection details from DATABASE_URL
    # Format: postgres://user:pass@host:port/dbname
    local db_host=$(echo "${db_url}" | sed -n 's|postgres://[^@]*@\([^:]*\):.*|\1|p')
    local db_port=$(echo "${db_url}" | sed -n 's|postgres://[^@]*@[^:]*:\([0-9]*\)/.*|\1|p')
    local db_name=$(echo "${db_url}" | sed -n 's|postgres://[^@]*@[^:]*:[0-9]*/\([^?]*\).*|\1|p')
    local db_user=$(echo "${db_url}" | sed -n 's|postgres://\([^:]*\):.*|\1|p')
    local db_pass=$(echo "${db_url}" | sed -n 's|postgres://[^:]*:\([^@]*\)@.*|\1|p')

    local backup_file="${BACKUP_DIR}/${module_name}_${TIMESTAMP}.sql"

    # Use pg_dump with connection parameters
    PGPASSWORD="${db_pass}" pg_dump \
        -h "${db_host}" \
        -p "${db_port}" \
        -U "${db_user}" \
        -d "${db_name}" \
        --format=plain \
        --no-owner \
        --no-privileges \
        --clean \
        --if-exists \
        > "${backup_file}" 2>&1

    if [ $? -eq 0 ]; then
        # Compress the backup
        gzip "${backup_file}"
        local compressed_size=$(du -h "${backup_file}.gz" | cut -f1)
        log_info "✓ ${module_name} backup complete: ${backup_file}.gz (${compressed_size})"
    else
        log_error "✗ ${module_name} backup failed"
        return 1
    fi
}

# Backup all module databases
FAILED_BACKUPS=0

backup_database "ar" "AR_DATABASE_URL" || ((FAILED_BACKUPS++))
backup_database "payments" "PAYMENTS_DATABASE_URL" || ((FAILED_BACKUPS++))
backup_database "subscriptions" "SUBSCRIPTIONS_DATABASE_URL" || ((FAILED_BACKUPS++))
backup_database "gl" "GL_DATABASE_URL" || ((FAILED_BACKUPS++))
backup_database "notifications" "NOTIFICATIONS_DATABASE_URL" || true  # Optional

# Create backup manifest
cat > "${BACKUP_DIR}/manifest.json" <<EOF
{
  "timestamp": "${TIMESTAMP}",
  "backup_dir": "${BACKUP_DIR}",
  "modules": ["ar", "payments", "subscriptions", "gl", "notifications"],
  "format": "pg_dump plain SQL (gzipped)",
  "version": "1.0.0",
  "failed_count": ${FAILED_BACKUPS}
}
EOF

log_info "Backup manifest created: ${BACKUP_DIR}/manifest.json"

# Summary
log_info "========================================="
log_info "Backup process complete"
log_info "Total modules backed up: $((5 - FAILED_BACKUPS))"
log_info "Failed backups: ${FAILED_BACKUPS}"
log_info "Backup location: ${BACKUP_DIR}"
log_info "========================================="

if [ ${FAILED_BACKUPS} -gt 0 ]; then
    log_error "Some backups failed. Check ${LOG_FILE} for details."
    exit 1
fi

exit 0
