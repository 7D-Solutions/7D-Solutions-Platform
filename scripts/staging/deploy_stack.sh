#!/usr/bin/env bash
# deploy_stack.sh — Apply a pinned image tag set to the staging VPS.
#
# Pulls the specified immutable images from the registry and restarts the
# staging Docker Compose stacks without rebuilding.  This is the CI/CD deploy
# path; deploy_compose.sh (--build) is the local-source dev path.
#
# Usage:
#   bash scripts/staging/deploy_stack.sh --tag v0.5.0-abc1234
#   bash scripts/staging/deploy_stack.sh --tag v0.5.0-abc1234 --dry-run
#   bash scripts/staging/deploy_stack.sh --tag v0.5.0-abc1234 --skip-smoke
#
# Required environment variables (set via secrets in CI):
#   STAGING_HOST        VPS hostname or IP
#   STAGING_USER        SSH user (must have docker access)
#   STAGING_REPO_PATH   Repo checkout path on VPS  (default: /opt/7d-platform)
#   IMAGE_REGISTRY      Registry prefix             (default: 7dsolutions)
#
# Optional:
#   STAGING_SSH_PORT    SSH port                    (default: 22)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
TAG=""
DRY_RUN=false
SKIP_SMOKE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag)       TAG="$2";   shift 2 ;;
        --dry-run)   DRY_RUN=true; shift ;;
        --skip-smoke) SKIP_SMOKE=true; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$TAG" ]]; then
    echo "ERROR: --tag <tag> is required." >&2
    echo "Example: bash scripts/staging/deploy_stack.sh --tag v0.5.0-abc1234" >&2
    exit 1
fi

# Refuse to deploy 'latest' — immutable tags only.
if [[ "$TAG" == "latest" || "$TAG" == *":latest" ]]; then
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
# Services — must stay in sync with build_images.sh / push_images.sh
# ---------------------------------------------------------------------------
declare -a SERVICES=(
    "control-plane"
    "identity-auth"
    "ttp"
    "ar"
    "payments"
    "tenant-control-plane-ui"
)

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
# Step 1: Write IMAGE_TAG to VPS .deploy.env
# ---------------------------------------------------------------------------
banner "Setting IMAGE_TAG=${TAG} on VPS"
DEPLOY_ENV_PATH="${STAGING_REPO_PATH}/.deploy.env"
run_remote "echo 'IMAGE_TAG=${TAG}' > ${DEPLOY_ENV_PATH} && echo 'REGISTRY=${REGISTRY}' >> ${DEPLOY_ENV_PATH}"
log "Wrote .deploy.env on VPS"

# ---------------------------------------------------------------------------
# Step 2: Pull images from registry with the pinned tag
# ---------------------------------------------------------------------------
banner "Pulling images (tag: ${TAG})"
for svc in "${SERVICES[@]}"; do
    full_image="${REGISTRY}/${svc}:${TAG}"
    log "  docker pull ${full_image}"
    run_remote "docker pull ${full_image}"
done

# ---------------------------------------------------------------------------
# Step 3: Apply images — restart stacks without rebuild
# ---------------------------------------------------------------------------
banner "Restarting stacks (IMAGE_TAG=${TAG})"

# Data stack (NATS + Postgres) — only restart if config changed, not on every deploy
# to avoid losing in-flight data. We use --no-recreate for data services.
run_remote "docker compose -f docker-compose.data.yml up -d --no-recreate"

# Platform stack (auth, control-plane)
run_remote "IMAGE_TAG=${TAG} docker compose -f docker-compose.platform.yml up -d --no-build --pull never"

# Backend modules stack
run_remote "IMAGE_TAG=${TAG} docker compose up -d --no-build --pull never"

# Frontend stack (TCP UI)
run_remote "IMAGE_TAG=${TAG} docker compose -f docker-compose.frontend.yml up -d --no-build --pull never"

# ---------------------------------------------------------------------------
# Step 4: Record deployment
# ---------------------------------------------------------------------------
DEPLOY_LOG="${STAGING_REPO_PATH}/.staging-deployments"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RECORD="${TIMESTAMP} tag=${TAG} registry=${REGISTRY}"
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
log "Deploy complete: tag=${TAG}"
