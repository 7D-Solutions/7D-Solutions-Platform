#!/usr/bin/env bash
# install_backup_timer.sh — Install scheduled daily backups via systemd timer or cron.
#
# Installs a daily backup pipeline that runs (in order):
#   1. backup_all_dbs.sh  — dump all Postgres databases
#   2. backup_ship.sh     — upload to off-host storage
#   3. backup_prune.sh    — enforce retention policy
#
# Systemd mode (preferred):
#   Installs /etc/systemd/system/7d-backup.service and 7d-backup.timer.
#   Timer fires daily at BACKUP_HOUR:00 UTC with Persistent=true (runs missed jobs on boot).
#
# Cron fallback:
#   If systemd is not detected, installs /etc/cron.d/7d-backup for the root user.
#
# Usage (must run as root):
#   sudo bash scripts/production/install_backup_timer.sh
#   sudo bash scripts/production/install_backup_timer.sh --repo-path /opt/7d-platform
#   sudo bash scripts/production/install_backup_timer.sh --dry-run
#
# Options:
#   --repo-path PATH   Absolute path to the platform repo on this VPS
#                      (default: /opt/7d-platform)
#   --hour HOUR        UTC hour to run backups, 0-23 (default: 2)
#   --dry-run          Print what would be installed without writing any files
#
# Exit: 0 = installed successfully. Non-zero = failure.

set -euo pipefail

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
REPO_PATH="/opt/7d-platform"
BACKUP_HOUR="2"
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo-path) REPO_PATH="$2"; shift 2 ;;
        --hour)      BACKUP_HOUR="$2"; shift 2 ;;
        --dry-run)   DRY_RUN=true; shift ;;
        *) echo "[install_backup_timer] ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Derived paths
# ---------------------------------------------------------------------------
SCRIPTS_DIR="${REPO_PATH}/scripts/production"
BACKUP_ALL="${SCRIPTS_DIR}/backup_all_dbs.sh"
BACKUP_SHIP="${SCRIPTS_DIR}/backup_ship.sh"
BACKUP_PRUNE="${SCRIPTS_DIR}/backup_prune.sh"

log()  { echo "[install_backup_timer] $*"; }
fail() { echo "[install_backup_timer] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Root check
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "false" ]] && [[ "$(id -u)" != "0" ]]; then
    fail "Must run as root. Use: sudo bash install_backup_timer.sh"
fi

# ---------------------------------------------------------------------------
# Preflight: verify backup scripts exist
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "false" ]]; then
    for _script in "$BACKUP_ALL" "$BACKUP_SHIP" "$BACKUP_PRUNE"; do
        if [[ ! -f "$_script" ]]; then
            fail "Script not found: $_script — deploy the repo to $REPO_PATH first"
        fi
    done
fi

log "Repository path: $REPO_PATH"
log "Backup hour:     ${BACKUP_HOUR}:00 UTC daily"
[[ "$DRY_RUN" == "true" ]] && log "Mode:            DRY RUN (no files written)"
log ""

# ---------------------------------------------------------------------------
# Install via systemd
# ---------------------------------------------------------------------------
install_systemd() {
    local service_file="/etc/systemd/system/7d-backup.service"
    local timer_file="/etc/systemd/system/7d-backup.timer"

    local service_content
    service_content="[Unit]
Description=7D Solutions Platform — daily database backup
Documentation=https://github.com/7d-solutions/platform
After=docker.service
Requires=docker.service

[Service]
Type=oneshot
ExecStart=/bin/bash ${BACKUP_ALL}
ExecStartPost=/bin/bash ${BACKUP_SHIP}
ExecStartPost=/bin/bash ${BACKUP_PRUNE}
StandardOutput=journal
StandardError=journal
SyslogIdentifier=7d-backup

[Install]
WantedBy=multi-user.target"

    local timer_content
    timer_content="[Unit]
Description=7D Solutions Platform — daily backup timer
Documentation=https://github.com/7d-solutions/platform

[Timer]
OnCalendar=*-*-* ${BACKUP_HOUR}:00:00 UTC
Persistent=true
Unit=7d-backup.service

[Install]
WantedBy=timers.target"

    if [[ "$DRY_RUN" == "true" ]]; then
        log "[DRY-RUN] Would write: $service_file"
        echo "--- $service_file ---"
        echo "$service_content"
        echo ""
        log "[DRY-RUN] Would write: $timer_file"
        echo "--- $timer_file ---"
        echo "$timer_content"
        echo ""
        log "[DRY-RUN] Would run: systemctl daemon-reload && systemctl enable --now 7d-backup.timer"
        return 0
    fi

    printf '%s\n' "$service_content" > "$service_file"
    log "Written: $service_file"

    printf '%s\n' "$timer_content" > "$timer_file"
    log "Written: $timer_file"

    systemctl daemon-reload
    systemctl enable --now 7d-backup.timer

    log ""
    log "Systemd timer installed and enabled."
    log ""
    log "Useful commands:"
    log "  systemctl status 7d-backup.timer          — check timer status"
    log "  systemctl status 7d-backup.service        — check last run"
    log "  journalctl -u 7d-backup.service -n 100    — view backup logs"
    log "  systemctl start 7d-backup.service         — run backup immediately"
    log "  systemctl list-timers 7d-backup.timer     — show next scheduled run"
}

# ---------------------------------------------------------------------------
# Install via cron (fallback)
# ---------------------------------------------------------------------------
install_cron() {
    local cron_file="/etc/cron.d/7d-backup"
    local log_file="/var/log/7d-backup.log"

    local cron_content
    cron_content="# 7D Solutions Platform — daily database backup
# Installed by install_backup_timer.sh
# Runs at ${BACKUP_HOUR}:00 UTC daily as root.
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/sbin:/bin:/usr/sbin:/usr/bin

0 ${BACKUP_HOUR} * * * root /bin/bash ${BACKUP_ALL} && /bin/bash ${BACKUP_SHIP} && /bin/bash ${BACKUP_PRUNE} >> ${log_file} 2>&1"

    if [[ "$DRY_RUN" == "true" ]]; then
        log "[DRY-RUN] Would write: $cron_file"
        echo "--- $cron_file ---"
        echo "$cron_content"
        echo ""
        return 0
    fi

    printf '%s\n' "$cron_content" > "$cron_file"
    chmod 0644 "$cron_file"
    log "Written: $cron_file"

    log ""
    log "Cron job installed."
    log ""
    log "Useful commands:"
    log "  cat $cron_file                   — view installed cron job"
    log "  tail -f $log_file                — follow backup logs"
    log "  bash $BACKUP_ALL                 — run backup immediately"
}

# ---------------------------------------------------------------------------
# Detect scheduler and install
# ---------------------------------------------------------------------------
if [[ -d /run/systemd/system ]]; then
    log "systemd detected — installing systemd service + timer"
    install_systemd
elif [[ -d /etc/cron.d ]]; then
    log "systemd not available — falling back to cron"
    install_cron
else
    fail "Neither systemd (/run/systemd/system) nor cron (/etc/cron.d) found — cannot schedule backups"
fi

log ""
log "Backup pipeline configured:"
log "  1. backup_all_dbs.sh  — dumps all DBs to BACKUP_DIR (default: /var/backups/7d-platform)"
log "  2. backup_ship.sh     — uploads to off-host storage (BACKUP_SHIP_METHOD: s3 or scp)"
log "  3. backup_prune.sh    — enforces retention (BACKUP_RETAIN_DAILY/WEEKLY)"
log ""
log "Set backup destination and credentials via /etc/7d/production/secrets.env before first run."
log "See scripts/production/env.example for the required variables."
exit 0
