#!/usr/bin/env bash
# backup_ship.sh — Copy the latest (or a specified) local backup to off-host storage.
#
# Supports two shipping methods:
#   s3   — Upload to an S3-compatible object store using the AWS CLI.
#           Works with AWS S3, DigitalOcean Spaces, Backblaze B2, MinIO, etc.
#   scp  — Copy via SCP/SSH to a remote host.
#
# Usage:
#   bash scripts/production/backup_ship.sh
#   bash scripts/production/backup_ship.sh --backup-dir /var/backups/7d-platform/2026-02-21_02-00-01
#
# Options:
#   --backup-dir PATH   Specific backup directory to ship (default: latest in BACKUP_DIR)
#
# ---- S3 environment variables ----
#   BACKUP_SHIP_METHOD        "s3" or "scp" (default: s3)
#   BACKUP_S3_BUCKET          S3 bucket name                         (required for s3)
#   BACKUP_S3_PREFIX          Key prefix within bucket               (default: backups/7d-platform)
#   BACKUP_S3_ENDPOINT_URL    Custom endpoint for S3-compatible APIs (optional)
#                             e.g. https://nyc3.digitaloceanspaces.com
#   AWS_ACCESS_KEY_ID         Access key (or use IAM instance profile)
#   AWS_SECRET_ACCESS_KEY     Secret key
#   AWS_DEFAULT_REGION        Region                                 (default: us-east-1)
#
# ---- SCP environment variables ----
#   BACKUP_SCP_HOST           Remote hostname or IP                  (required for scp)
#   BACKUP_SCP_USER           Remote user                            (default: backup)
#   BACKUP_SCP_PORT           Remote SSH port                        (default: 22)
#   BACKUP_SCP_PATH           Remote base path                       (default: /var/backups/7d-platform)
#   BACKUP_SCP_KEY            Path to SSH private key                (default: ~/.ssh/id_ed25519)
#
# ---- Common ----
#   BACKUP_DIR                Root local backup directory            (default: /var/backups/7d-platform)
#
# Exit: 0 = shipped successfully. Non-zero = failure.

set -euo pipefail

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
OVERRIDE_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backup-dir) OVERRIDE_DIR="$2"; shift 2 ;;
        *) echo "[backup_ship] ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
BACKUP_DIR="${BACKUP_DIR:-/var/backups/7d-platform}"
SHIP_METHOD="${BACKUP_SHIP_METHOD:-s3}"

log()  { echo "[backup_ship] $*"; }
fail() { echo "[backup_ship] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Resolve target backup directory
# ---------------------------------------------------------------------------
if [[ -n "$OVERRIDE_DIR" ]]; then
    TARGET_DIR="$OVERRIDE_DIR"
else
    TARGET_DIR="$(
        ls -1d "${BACKUP_DIR}"/????-??-??_??-??-?? 2>/dev/null \
        | sort -r \
        | head -1 \
        || true
    )"
fi

if [[ -z "$TARGET_DIR" || ! -d "$TARGET_DIR" ]]; then
    fail "No backup directory to ship (BACKUP_DIR=${BACKUP_DIR})"
fi

BACKUP_NAME="$(basename "$TARGET_DIR")"
MANIFEST="${TARGET_DIR}/MANIFEST.txt"

log "Backup to ship:  $BACKUP_NAME"
log "Ship method:     $SHIP_METHOD"

if [[ ! -f "$MANIFEST" ]]; then
    log "WARN: No MANIFEST.txt found in $TARGET_DIR — proceeding without manifest verification"
fi

# ---------------------------------------------------------------------------
# S3 shipping
# ---------------------------------------------------------------------------
ship_s3() {
    local bucket="${BACKUP_S3_BUCKET:?BACKUP_S3_BUCKET must be set for BACKUP_SHIP_METHOD=s3}"
    local prefix="${BACKUP_S3_PREFIX:-backups/7d-platform}"
    local region="${AWS_DEFAULT_REGION:-us-east-1}"
    local s3_dest="s3://${bucket}/${prefix}/${BACKUP_NAME}/"

    if ! command -v aws >/dev/null 2>&1; then
        fail "aws CLI not found — install it (pip install awscli) or use BACKUP_SHIP_METHOD=scp"
    fi

    local -a aws_opts=(--region "$region")
    if [[ -n "${BACKUP_S3_ENDPOINT_URL:-}" ]]; then
        aws_opts+=(--endpoint-url "$BACKUP_S3_ENDPOINT_URL")
        log "S3 endpoint:     $BACKUP_S3_ENDPOINT_URL"
    fi

    log "Uploading to:    $s3_dest"

    aws s3 sync "${aws_opts[@]}" \
        --no-progress \
        --storage-class STANDARD_IA \
        "${TARGET_DIR}/" \
        "$s3_dest"

    # Verify: remote file count must be >= local file count
    local remote_count
    remote_count="$(aws s3 ls "${aws_opts[@]}" "$s3_dest" | grep -c '\.sql\.gz\|MANIFEST' || true)"
    local local_count
    local_count="$(find "$TARGET_DIR" -maxdepth 1 -type f | wc -l | tr -d '[:space:]')"

    log "Verification:    remote=${remote_count} files, local=${local_count} files"
    if [[ "$remote_count" -lt "$local_count" ]]; then
        fail "Remote file count (${remote_count}) < local file count (${local_count}) — ship may be incomplete"
    fi

    log "S3 ship complete: $s3_dest"
}

# ---------------------------------------------------------------------------
# SCP shipping
# ---------------------------------------------------------------------------
ship_scp() {
    local host="${BACKUP_SCP_HOST:?BACKUP_SCP_HOST must be set for BACKUP_SHIP_METHOD=scp}"
    local user="${BACKUP_SCP_USER:-backup}"
    local port="${BACKUP_SCP_PORT:-22}"
    local remote_path="${BACKUP_SCP_PATH:-/var/backups/7d-platform}"
    local key="${BACKUP_SCP_KEY:-${HOME}/.ssh/id_ed25519}"

    local -a ssh_opts=(-o StrictHostKeyChecking=no -o BatchMode=yes -p "$port")
    local -a scp_opts=(-r -P "$port" -o StrictHostKeyChecking=no -o BatchMode=yes)

    if [[ -f "$key" ]]; then
        ssh_opts+=(-i "$key")
        scp_opts+=(-i "$key")
    fi

    local remote_dir="${remote_path}/${BACKUP_NAME}"

    log "Copying to:      ${user}@${host}:${remote_dir}"

    # Ensure remote base directory exists
    ssh "${ssh_opts[@]}" "${user}@${host}" "mkdir -p '${remote_dir}'"

    # Copy all files from the backup directory
    scp "${scp_opts[@]}" "${TARGET_DIR}/"* "${user}@${host}:${remote_dir}/"

    # Verify: remote file count must be >= local file count
    local remote_count
    remote_count="$(ssh "${ssh_opts[@]}" "${user}@${host}" \
        "ls -1 '${remote_dir}/' 2>/dev/null | wc -l" | tr -d '[:space:]')"
    local local_count
    local_count="$(find "$TARGET_DIR" -maxdepth 1 -type f | wc -l | tr -d '[:space:]')"

    log "Verification:    remote=${remote_count} files, local=${local_count} files"
    if [[ "$remote_count" -lt "$local_count" ]]; then
        fail "Remote file count (${remote_count}) < local file count (${local_count}) — ship may be incomplete"
    fi

    log "SCP ship complete: ${user}@${host}:${remote_dir}"
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
case "$SHIP_METHOD" in
    s3)  ship_s3  ;;
    scp) ship_scp ;;
    *)   fail "Unknown BACKUP_SHIP_METHOD: '${SHIP_METHOD}' (valid values: s3, scp)" ;;
esac

log ""
log "Backup shipped successfully: $BACKUP_NAME"
exit 0
