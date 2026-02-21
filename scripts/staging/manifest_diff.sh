#!/usr/bin/env bash
# manifest_diff.sh — Compare MODULE-MANIFEST.md against the running staging stack.
#
# SSH into the staging VPS, inspect each expected container's running image,
# and compare against what the manifest declares. Exits non-zero on any mismatch.
#
# Pending entries (SHA = "—" or tag contains "{sha}") are skipped with a warning.
#
# Usage:
#   bash scripts/staging/manifest_diff.sh
#
# Environment (required for SSH):
#   STAGING_HOST      VPS hostname or IP
#   STAGING_USER      SSH user (must have docker access)
#   STAGING_SSH_PORT  SSH port (default: 22)
#
# Optional:
#   MANIFEST_FILE     Override manifest path (default: deploy/staging/MODULE-MANIFEST.md)
#   IMAGE_REGISTRY    Override registry prefix (default: 7dsolutions)
#   DRY_RUN           Set to "true" to print SSH commands without executing

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_FILE="${MANIFEST_FILE:-${REPO_ROOT}/deploy/staging/MODULE-MANIFEST.md}"
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"
STAGING_HOST="${STAGING_HOST:-}"
STAGING_USER="${STAGING_USER:-}"
STAGING_SSH_PORT="${STAGING_SSH_PORT:-22}"
DRY_RUN="${DRY_RUN:-false}"

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
    exit 1
fi

if [[ -z "$STAGING_HOST" || -z "$STAGING_USER" ]]; then
    echo "ERROR: STAGING_HOST and STAGING_USER must be set." >&2
    exit 1
fi

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[manifest_diff] $*"; }
warn()   { echo "[manifest_diff] WARN: $*" >&2; }

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${STAGING_SSH_PORT}"
SSH_TARGET="${STAGING_USER}@${STAGING_HOST}"

run_remote() {
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  [DRY-RUN] ssh ${SSH_TARGET}: $*"
        echo ""
        return 0
    fi
    ssh $SSH_OPTS "$SSH_TARGET" "$*"
}

# ---------------------------------------------------------------------------
# Mapping: manifest canonical name → container name(s) on VPS
# Add new entries here when new services are added to the manifest.
# ---------------------------------------------------------------------------
declare -A CONTAINER_MAP=(
    ["control-plane"]="7d-control-plane"
    ["identity-auth"]="7d-auth-1"
    ["ttp"]="7d-ttp"
    ["ar"]="7d-ar"
    ["payments"]="7d-payments"
    ["tenant-control-plane-ui"]="7d-tcp-ui"
)

# ---------------------------------------------------------------------------
# Parse the manifest table
# Output: canonical_name|full_image_tag|sha_field  (one per line)
# ---------------------------------------------------------------------------
parse_manifest() {
    grep '^|' "$MANIFEST_FILE" \
        | grep -v '^| Service\|^|---\|^| ---' \
        | awk -F'|' '{
            canonical = $3
            sha_field = $5
            full_tag  = $6
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", canonical)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", sha_field)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", full_tag)
            gsub(/^`|`$/, "", canonical)
            gsub(/^`|`$/, "", full_tag)
            if (length(canonical) > 0 && length(full_tag) > 0) {
                print canonical "|" full_tag "|" sha_field
            }
        }'
}

# ---------------------------------------------------------------------------
# Verify SSH connectivity
# ---------------------------------------------------------------------------
banner "Checking SSH connectivity to ${SSH_TARGET}"
if [[ "$DRY_RUN" != "true" ]]; then
    if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
        echo "ERROR: Cannot reach ${SSH_TARGET} via SSH." >&2
        exit 1
    fi
    log "SSH connectivity: OK"
else
    log "SSH check: [DRY-RUN skipped]"
fi

# ---------------------------------------------------------------------------
# Diff loop
# ---------------------------------------------------------------------------
banner "Comparing manifest vs running containers on ${STAGING_HOST}"

MATCH=0
MISMATCH=0
MISSING=0
PENDING=0

# Header for output table
printf "%-32s  %-50s  %-50s  %s\n" "SERVICE" "EXPECTED (manifest)" "ACTUAL (running)" "STATUS"
printf "%-32s  %-50s  %-50s  %s\n" "-------" "-------------------" "----------------" "------"

while IFS='|' read -r canonical expected_image sha_field; do
    [[ -z "$canonical" ]] && continue

    # Skip pending entries
    if [[ "$sha_field" == "—" || "$expected_image" == *"{sha}"* ]]; then
        warn "Skipping pending entry: ${canonical} (${expected_image})"
        PENDING=$((PENDING + 1))
        continue
    fi

    # Look up container name
    container="${CONTAINER_MAP[$canonical]:-}"
    if [[ -z "$container" ]]; then
        printf "%-32s  %-50s  %-50s  %s\n" \
            "$canonical" "$expected_image" "(no container mapping)" "WARN"
        warn "No container mapping for canonical name: ${canonical}"
        continue
    fi

    # Query the running container's image
    if [[ "$DRY_RUN" == "true" ]]; then
        actual_image="[DRY-RUN:${container}]"
    else
        actual_image="$(run_remote \
            "docker inspect --format '{{.Config.Image}}' ${container} 2>/dev/null || echo 'NOT_RUNNING'")"
        actual_image="${actual_image%%[[:space:]]*}"  # trim whitespace
    fi

    if [[ "$actual_image" == "NOT_RUNNING" ]]; then
        printf "%-32s  %-50s  %-50s  %s\n" \
            "$canonical" "$expected_image" "(container not running)" "FAIL"
        MISSING=$((MISSING + 1))
    elif [[ "$actual_image" == "$expected_image" ]]; then
        printf "%-32s  %-50s  %-50s  %s\n" \
            "$canonical" "$expected_image" "$actual_image" "OK"
        MATCH=$((MATCH + 1))
    else
        printf "%-32s  %-50s  %-50s  %s\n" \
            "$canonical" "$expected_image" "$actual_image" "MISMATCH"
        MISMATCH=$((MISMATCH + 1))
    fi
done < <(parse_manifest)

echo ""
log "Results: ${MATCH} OK / ${MISMATCH} MISMATCH / ${MISSING} NOT_RUNNING / ${PENDING} PENDING"

if [[ $MISMATCH -gt 0 || $MISSING -gt 0 ]]; then
    echo ""
    TOTAL_FAIL=$((MISMATCH + MISSING))
    echo "ERROR: ${TOTAL_FAIL} service(s) do not match the manifest." >&2
    if [[ $MISMATCH -gt 0 ]]; then
        echo "       MISMATCH: running image differs from pinned manifest tag." >&2
    fi
    if [[ $MISSING -gt 0 ]]; then
        echo "       NOT_RUNNING: container is not up on the staging host." >&2
    fi
    echo "       Re-run promote.yml with the correct manifest or investigate the VPS." >&2
    exit 1
fi

log "Diff passed — running images match the manifest."
