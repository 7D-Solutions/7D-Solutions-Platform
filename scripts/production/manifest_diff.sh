#!/usr/bin/env bash
# manifest_diff.sh — Compare deploy/production/MODULE-MANIFEST.md against the running production stack.
#
# SSH into the production VPS, inspect each expected container's running image,
# and compare against what the manifest declares. Exits non-zero on any mismatch.
#
# Production ports are firewalled; this script uses docker inspect (not HTTP)
# to determine running images — no inbound port access required.
#
# Pending entries (SHA = "—" or tag contains "{sha}") are skipped with a warning.
#
# Usage:
#   bash scripts/production/manifest_diff.sh
#   bash scripts/production/manifest_diff.sh deploy/production/MODULE-MANIFEST.md
#
# Environment (required for SSH):
#   PROD_HOST         VPS hostname or IP
#   PROD_USER         SSH user (must have docker access)
#   PROD_SSH_PORT     SSH port (default: 22)
#
# Optional:
#   MANIFEST_FILE     Override manifest path (default: deploy/production/MODULE-MANIFEST.md)
#   IMAGE_REGISTRY    Override registry prefix (default: 7dsolutions)
#   DRY_RUN           Set to "true" to print SSH commands without executing

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_FILE="${MANIFEST_FILE:-${REPO_ROOT}/deploy/production/MODULE-MANIFEST.md}"
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"
PROD_HOST="${PROD_HOST:-}"
PROD_USER="${PROD_USER:-}"
PROD_SSH_PORT="${PROD_SSH_PORT:-22}"
DRY_RUN="${DRY_RUN:-false}"

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
        --*)       echo "Unknown argument: $arg" >&2; exit 1 ;;
        *)         MANIFEST_FILE="$arg" ;;
    esac
done

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
    exit 1
fi

if [[ -z "$PROD_HOST" || -z "$PROD_USER" ]]; then
    echo "ERROR: PROD_HOST and PROD_USER must be set." >&2
    exit 1
fi

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[manifest_diff:prod] $*"; }
warn()   { echo "[manifest_diff:prod] WARN: $*" >&2; }

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${PROD_SSH_PORT}"
SSH_TARGET="${PROD_USER}@${PROD_HOST}"

run_remote() {
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  [DRY-RUN] ssh ${SSH_TARGET}: $*"
        echo ""
        return 0
    fi
    ssh $SSH_OPTS "$SSH_TARGET" "$*"
}

# ---------------------------------------------------------------------------
# Mapping: manifest canonical name → container name on production VPS
# Must stay in sync with the docker-compose files.
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
banner "Comparing production manifest vs running containers on ${PROD_HOST}"

MATCH=0
MISMATCH=0
MISSING=0
PENDING=0

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

    # Query the running container's image via docker inspect
    if [[ "$DRY_RUN" == "true" ]]; then
        actual_image="[DRY-RUN:${container}]"
    else
        actual_image="$(run_remote \
            "docker inspect --format '{{.Config.Image}}' ${container} 2>/dev/null || echo 'NOT_RUNNING'")"
        actual_image="${actual_image%%[[:space:]]*}"
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
    echo "ERROR: ${TOTAL_FAIL} service(s) do not match the production manifest." >&2
    if [[ $MISMATCH -gt 0 ]]; then
        echo "       MISMATCH: running image differs from the pinned manifest tag." >&2
        echo "       Re-run deploy_stack.sh to apply the correct manifest." >&2
    fi
    if [[ $MISSING -gt 0 ]]; then
        echo "       NOT_RUNNING: container is not up on the production host." >&2
        echo "       Check docker ps and service logs on the VPS." >&2
    fi
    exit 1
fi

log "Diff passed — running images match the production manifest."
