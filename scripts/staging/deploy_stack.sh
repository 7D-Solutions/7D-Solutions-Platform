#!/usr/bin/env bash
# deploy_stack.sh — Deploy staging stack from MODULE-MANIFEST.md (single source of truth).
#
# The manifest at deploy/staging/MODULE-MANIFEST.md defines exactly which image
# tags to deploy. No ad-hoc tag overrides are accepted in normal operation.
#
# Usage:
#   bash scripts/staging/deploy_stack.sh
#   bash scripts/staging/deploy_stack.sh --manifest <path>   # alternate manifest
#   bash scripts/staging/deploy_stack.sh --dry-run
#   bash scripts/staging/deploy_stack.sh --skip-smoke
#
# DEV-ONLY override (guarded — off by default):
#   DEPLOY_ALLOW_TAG_OVERRIDE=1 \
#     bash scripts/staging/deploy_stack.sh --tag-override v0.5.0-abc1234
#
# Required environment variables (set via secrets in CI):
#   STAGING_HOST        VPS hostname or IP
#   STAGING_USER        SSH user (must have docker access)
#
# Optional:
#   STAGING_REPO_PATH   Repo checkout path on VPS  (default: /opt/7d-platform)
#   STAGING_SSH_PORT    SSH port                    (default: 22)
#   IMAGE_REGISTRY      Registry prefix             (default: 7dsolutions)
#   MANIFEST_FILE       Override manifest path (same as --manifest flag)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
MANIFEST_FILE="${MANIFEST_FILE:-${REPO_ROOT}/deploy/staging/MODULE-MANIFEST.md}"
DRY_RUN=false
SKIP_SMOKE=false
TAG_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --manifest)      MANIFEST_FILE="$2"; shift 2 ;;
        --dry-run)       DRY_RUN=true; shift ;;
        --skip-smoke)    SKIP_SMOKE=true; shift ;;
        --tag-override)
            if [[ "${DEPLOY_ALLOW_TAG_OVERRIDE:-}" != "1" ]]; then
                echo "ERROR: --tag-override requires DEPLOY_ALLOW_TAG_OVERRIDE=1 (dev-only flag)." >&2
                echo "       For normal deploys, update deploy/staging/MODULE-MANIFEST.md instead." >&2
                exit 1
            fi
            TAG_OVERRIDE="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
    exit 1
fi

# Refuse to deploy 'latest' even via override
if [[ "$TAG_OVERRIDE" == "latest" || "$TAG_OVERRIDE" == *":latest" ]]; then
    echo "ERROR: Refusing to deploy 'latest' tag. Specify an immutable tag." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Connection config
# ---------------------------------------------------------------------------
STAGING_HOST="${STAGING_HOST:?ERROR: STAGING_HOST must be set}"
STAGING_USER="${STAGING_USER:-deploy}"
STAGING_REPO_PATH="${STAGING_REPO_PATH:-/opt/7d-platform}"
STAGING_SSH_PORT="${STAGING_SSH_PORT:-22}"
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${STAGING_SSH_PORT}"
SSH_TARGET="${STAGING_USER}@${STAGING_HOST}"

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[deploy_stack] $*"; }
warn()   { echo "[deploy_stack] WARN: $*" >&2; }

run_remote() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] ssh ${SSH_TARGET}: $*"
    else
        # shellcheck disable=SC2029
        ssh $SSH_OPTS "$SSH_TARGET" "cd ${STAGING_REPO_PATH} && $*"
    fi
}

run_local() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] $*"
    else
        "$@"
    fi
}

# ---------------------------------------------------------------------------
# Parse manifest table → arrays of canonical names and full image tags
# Output: "canonical|full_image_tag|sha_field" per line
# Skips header and separator rows.
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

# Convert canonical service name to env-var prefix: control-plane → CONTROL_PLANE
canonical_to_var() {
    echo "$1" | tr '[:lower:]-' '[:upper:]_'
}

# ---------------------------------------------------------------------------
# Load manifest entries into parallel arrays
# ---------------------------------------------------------------------------
banner "Loading manifest: ${MANIFEST_FILE}"

declare -a SVC_NAMES=()
declare -a SVC_IMAGES=()
PENDING_COUNT=0
RESOLVED_COUNT=0

while IFS='|' read -r canonical full_tag sha_field; do
    [[ -z "$canonical" ]] && continue

    if [[ "$sha_field" == "—" || "$full_tag" == *"{sha}"* ]]; then
        warn "Pending entry skipped: ${canonical} (${full_tag})"
        PENDING_COUNT=$((PENDING_COUNT + 1))
        continue
    fi

    if [[ -n "$TAG_OVERRIDE" ]]; then
        # Dev override: derive image from canonical name + override tag
        # Strip registry prefix if already in full_tag, rebuild with override tag
        image_name="${REGISTRY}/${canonical}"
        full_tag="${image_name}:${TAG_OVERRIDE}"
        warn "DEV OVERRIDE: ${canonical} → ${full_tag}"
    fi

    SVC_NAMES+=("$canonical")
    SVC_IMAGES+=("$full_tag")
    RESOLVED_COUNT=$((RESOLVED_COUNT + 1))
    log "  ${canonical} → ${full_tag}"
done < <(parse_manifest)

if [[ $RESOLVED_COUNT -eq 0 ]]; then
    if [[ $PENDING_COUNT -gt 0 ]]; then
        echo "ERROR: All ${PENDING_COUNT} manifest entries are pending (no resolved tags)." >&2
        echo "       Run 'bash scripts/staging/push_images.sh' and update the manifest first." >&2
    else
        echo "ERROR: No entries found in manifest: ${MANIFEST_FILE}" >&2
    fi
    exit 1
fi

log "Manifest loaded: ${RESOLVED_COUNT} resolved / ${PENDING_COUNT} pending"

if [[ -n "$TAG_OVERRIDE" ]]; then
    banner "DEV MODE — tag override active: ${TAG_OVERRIDE}"
    echo "  WARNING: This bypasses normal manifest versioning. Dev use only."
fi

# ---------------------------------------------------------------------------
# Preflight: verify SSH connectivity
# ---------------------------------------------------------------------------
banner "Preflight"
if ! $DRY_RUN; then
    if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
        echo "ERROR: Cannot reach ${SSH_TARGET} via SSH." >&2
        exit 1
    fi
    log "SSH connectivity: OK"
fi

# ---------------------------------------------------------------------------
# Step 1: Pull images from registry (per-service manifest tags)
# ---------------------------------------------------------------------------
banner "Pulling images from registry"
for i in "${!SVC_NAMES[@]}"; do
    svc="${SVC_NAMES[$i]}"
    img="${SVC_IMAGES[$i]}"
    log "  docker pull ${img}"
    run_remote "docker pull ${img}"
done

# ---------------------------------------------------------------------------
# Step 2: Write per-service image env file on VPS
# Format: {SVC_VAR}_IMAGE=<full_image_tag>
# Used by docker-compose files that reference ${SVC_IMAGE} variables.
# ---------------------------------------------------------------------------
banner "Writing deploy-images.env on VPS"
DEPLOY_IMAGES_ENV="${STAGING_REPO_PATH}/.deploy-images.env"

ENV_LINES=""
for i in "${!SVC_NAMES[@]}"; do
    var_prefix="$(canonical_to_var "${SVC_NAMES[$i]}")"
    ENV_LINES+="${var_prefix}_IMAGE=${SVC_IMAGES[$i]}\n"
done
# Also export REGISTRY for any compose files that still need it
ENV_LINES+="IMAGE_REGISTRY=${REGISTRY}\n"

if $DRY_RUN; then
    echo "  [DRY-RUN] Would write ${DEPLOY_IMAGES_ENV}:"
    echo -e "$ENV_LINES" | sed 's/^/    /'
else
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" \
        "printf '%b' '${ENV_LINES}' > ${DEPLOY_IMAGES_ENV}"
fi
log "Wrote ${DEPLOY_IMAGES_ENV}"

# ---------------------------------------------------------------------------
# Step 3: Apply images — restart stacks without rebuild
# All stacks pick up the env file which declares the per-service image vars.
# ---------------------------------------------------------------------------
banner "Restarting stacks (manifest-pinned images)"

# Data stack (NATS + Postgres) — no-recreate to preserve in-flight data
run_remote "docker compose -f docker-compose.data.yml up -d --no-recreate"

# Platform stack (auth, control-plane)
run_remote "docker compose --env-file ${DEPLOY_IMAGES_ENV} \
    -f docker-compose.platform.yml up -d --no-build --pull never"

# Backend modules stack
run_remote "docker compose --env-file ${DEPLOY_IMAGES_ENV} \
    up -d --no-build --pull never"

# Frontend stack (TCP UI)
run_remote "docker compose --env-file ${DEPLOY_IMAGES_ENV} \
    -f docker-compose.frontend.yml up -d --no-build --pull never"

# ---------------------------------------------------------------------------
# Step 4: Archive manifest snapshot + record deployment
# ---------------------------------------------------------------------------
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
SNAPSHOT_DIR="${STAGING_REPO_PATH}/.manifest-snapshots"
SNAPSHOT_NAME="${TIMESTAMP}-MODULE-MANIFEST.md"
DEPLOY_LOG="${STAGING_REPO_PATH}/.staging-deployments"

# Compute manifest identity for the log
MANIFEST_HASH="$(sha256sum "$MANIFEST_FILE" 2>/dev/null | cut -c1-12 || echo "unknown")"
if git -C "$REPO_ROOT" rev-parse HEAD >/dev/null 2>&1; then
    MANIFEST_GIT_SHA="$(git -C "$REPO_ROOT" log --format="%H" -1 \
        -- "deploy/staging/MODULE-MANIFEST.md" 2>/dev/null | head -1 | cut -c1-12 || echo "unknown")"
else
    MANIFEST_GIT_SHA="none"
fi

# Push manifest to VPS archive so rollback can restore it
if ! $DRY_RUN; then
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" "mkdir -p ${SNAPSHOT_DIR}"
    scp -q -P "${STAGING_SSH_PORT}" -o StrictHostKeyChecking=no \
        "$MANIFEST_FILE" "${SSH_TARGET}:${SNAPSHOT_DIR}/${SNAPSHOT_NAME}"
    log "Manifest archived to VPS: ${SNAPSHOT_DIR}/${SNAPSHOT_NAME}"
else
    echo "  [DRY-RUN] scp ${MANIFEST_FILE} → ${SSH_TARGET}:${SNAPSHOT_DIR}/${SNAPSHOT_NAME}"
fi

RECORD="${TIMESTAMP} manifest_hash=${MANIFEST_HASH} manifest_git_sha=${MANIFEST_GIT_SHA} snapshot=${SNAPSHOT_NAME}"
run_remote "echo '${RECORD}' >> ${DEPLOY_LOG} && tail -20 ${DEPLOY_LOG}"
log "Deployment recorded: ${RECORD}"

# ---------------------------------------------------------------------------
# Step 5: Show running containers
# ---------------------------------------------------------------------------
banner "Running containers"
run_remote "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}'"

# ---------------------------------------------------------------------------
# Step 6: Smoke checks (optional)
# ---------------------------------------------------------------------------
if $SKIP_SMOKE; then
    log "Smoke checks skipped (--skip-smoke)"
else
    banner "Smoke checks"
    echo "Waiting 15s for services to become healthy..."
    if ! $DRY_RUN; then sleep 15; fi

    HOST="$STAGING_HOST"
    FAILED=0

    declare -A HEALTH_ENDPOINTS=(
        ["auth"]="http://${HOST}:8080/api/health"
        ["control-plane"]="http://${HOST}:8091/api/ready"
        ["ar"]="http://${HOST}:8086/api/health"
        ["payments"]="http://${HOST}:8088/api/health"
        ["ttp"]="http://${HOST}:8100/api/health"
        ["tcp-ui"]="http://${HOST}:3000/login"
    )

    for svc in "${!HEALTH_ENDPOINTS[@]}"; do
        url="${HEALTH_ENDPOINTS[$svc]}"
        if $DRY_RUN; then
            echo "  [DRY-RUN] curl $url"
        else
            status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$url" 2>/dev/null || echo "000")
            if [[ "$status" == "200" ]]; then
                echo "  ✓ ${svc} (HTTP ${status})"
            else
                echo "  ✗ ${svc} — HTTP ${status} ($url)"
                FAILED=$((FAILED + 1))
            fi
        fi
    done

    echo ""
    if [[ $FAILED -eq 0 ]]; then
        log "All smoke checks passed."
    else
        log "WARNING: ${FAILED} service(s) failed smoke checks."
        exit 1
    fi
fi

echo ""
log "Deploy complete. Manifest: ${MANIFEST_FILE} (hash: ${MANIFEST_HASH}, git: ${MANIFEST_GIT_SHA})"
