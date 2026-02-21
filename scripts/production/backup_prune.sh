#!/usr/bin/env bash
# backup_prune.sh — Enforce retention policy on local 7D Platform backup directories.
#
# Retention policy:
#   Daily  — Keep the most recent BACKUP_RETAIN_DAILY unique calendar days (one backup per day).
#   Weekly — Keep one backup per ISO week for the next BACKUP_RETAIN_WEEKLY weeks beyond
#            the daily window (newest backup in each week is retained).
#   Older  — All other backup directories are deleted.
#
# Usage:
#   bash scripts/production/backup_prune.sh
#   DRY_RUN=true bash scripts/production/backup_prune.sh
#
# Optional environment:
#   BACKUP_DIR            Root backup directory (default: /var/backups/7d-platform)
#   BACKUP_RETAIN_DAILY   Daily backups to keep, one per day (default: 7)
#   BACKUP_RETAIN_WEEKLY  Weekly backups to keep beyond daily window (default: 4)
#   DRY_RUN               Set to "true" to print deletions without executing (default: false)
#
# Backup directories must be named YYYY-MM-DD_HH-MM-SS (as written by backup_all_dbs.sh).
#
# Exit: 0 = success. Non-zero = error.

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
BACKUP_DIR="${BACKUP_DIR:-/var/backups/7d-platform}"
RETAIN_DAILY="${BACKUP_RETAIN_DAILY:-7}"
RETAIN_WEEKLY="${BACKUP_RETAIN_WEEKLY:-4}"
DRY_RUN="${DRY_RUN:-false}"

log()  { echo "[backup_prune] $*"; }
fail() { echo "[backup_prune] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
if [[ ! -d "$BACKUP_DIR" ]]; then
    fail "Backup directory not found: $BACKUP_DIR"
fi

if ! command -v date >/dev/null 2>&1; then
    fail "date command not found"
fi

log "Backup directory:  $BACKUP_DIR"
log "Retain daily:      ${RETAIN_DAILY} day(s)"
log "Retain weekly:     ${RETAIN_WEEKLY} week(s) beyond daily window"
[[ "$DRY_RUN" == "true" ]] && log "Mode:              DRY RUN (no deletions)"

# ---------------------------------------------------------------------------
# Collect backup directories, newest first
# Glob pattern matches: YYYY-MM-DD_HH-MM-SS
# Sorted by name in reverse (ISO date format sorts lexicographically = chronologically).
# ---------------------------------------------------------------------------
mapfile -t ALL_BACKUPS < <(
    ls -1d "${BACKUP_DIR}"/????-??-??_??-??-?? 2>/dev/null \
    | sort -r \
    || true
)

TOTAL=${#ALL_BACKUPS[@]}
if [[ $TOTAL -eq 0 ]]; then
    log "No backup directories found — nothing to prune."
    exit 0
fi
log "Found ${TOTAL} backup directory(s)"
log ""

# ---------------------------------------------------------------------------
# Apply retention policy (iterate newest → oldest)
#
# daily_seen:  associative array keyed by YYYY-MM-DD  → 1 once that day is claimed
# weekly_seen: associative array keyed by GGGG-WW     → 1 once that week is claimed
# ---------------------------------------------------------------------------
declare -A daily_seen=()
declare -A weekly_seen=()
declare -a kept_daily=()
declare -a kept_weekly=()
declare -a to_delete=()

for _backup_dir in "${ALL_BACKUPS[@]}"; do
    _base="$(basename "$_backup_dir")"

    # Extract date: first 10 chars of YYYY-MM-DD_HH-MM-SS
    _date="${_base:0:10}"

    # Derive ISO year-week (GGGG-WW) using GNU date
    _week="$(date -d "$_date" +%G-%V 2>/dev/null)" || {
        log "WARN: Cannot parse date from '$_base' — skipping retention check"
        continue
    }

    if [[ -v daily_seen["$_date"] ]]; then
        # Another backup for this day is already kept — delete this duplicate
        to_delete+=("$_backup_dir")

    elif [[ ${#daily_seen[@]} -lt $RETAIN_DAILY ]]; then
        # Within daily window: keep the newest backup seen for this day
        daily_seen["$_date"]=1
        kept_daily+=("$_backup_dir")

    elif [[ -v weekly_seen["$_week"] ]]; then
        # Another backup for this ISO week is already kept — delete this duplicate
        to_delete+=("$_backup_dir")

    elif [[ ${#weekly_seen[@]} -lt $RETAIN_WEEKLY ]]; then
        # Within weekly window: keep the newest backup seen for this week
        weekly_seen["$_week"]=1
        kept_weekly+=("$_backup_dir")

    else
        # Outside both daily and weekly windows — delete
        to_delete+=("$_backup_dir")
    fi
done

# ---------------------------------------------------------------------------
# Report kept backups
# ---------------------------------------------------------------------------
log "Keeping ${#kept_daily[@]} daily backup(s):"
for _d in "${kept_daily[@]}"; do
    log "  [daily]  $(basename "$_d")"
done

log "Keeping ${#kept_weekly[@]} weekly backup(s):"
for _d in "${kept_weekly[@]}"; do
    log "  [weekly] $(basename "$_d")"
done

# ---------------------------------------------------------------------------
# Delete or dry-run
# ---------------------------------------------------------------------------
DELETE_COUNT=${#to_delete[@]}
if [[ $DELETE_COUNT -eq 0 ]]; then
    log ""
    log "Nothing to prune — all backups are within retention policy."
    exit 0
fi

log ""
log "Pruning ${DELETE_COUNT} backup(s) outside retention window:"
for _backup_dir in "${to_delete[@]}"; do
    if [[ "$DRY_RUN" == "true" ]]; then
        log "  [DRY-RUN] would delete: $(basename "$_backup_dir")"
    else
        log "  Deleting: $(basename "$_backup_dir")"
        rm -rf "$_backup_dir"
    fi
done

log ""
if [[ "$DRY_RUN" == "true" ]]; then
    log "Dry run complete — no backups were deleted."
else
    log "Pruning complete — ${DELETE_COUNT} backup(s) removed."
fi

exit 0
