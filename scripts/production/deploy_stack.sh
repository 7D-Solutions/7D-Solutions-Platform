#!/usr/bin/env bash
# deploy_stack.sh — Deploy a manifest-pinned image tag set to the production VPS.
#
# Reads deploy/production/MODULE-MANIFEST.md as the authoritative source of image
# tags.  Pulls the specified immutable images from the registry and restarts the
# production Docker Compose stacks without rebuilding.
#
# All external production ports are firewalled (UFW).  Smoke checks run via SSH
# on the VPS against localhost so no service port needs to be open to the internet.
#
# Usage:
#   bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md
#   bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md --dry-run
#   bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md --skip-smoke
#
# Alternatively, supply a tag directly (bypasses manifest — emergency only):
#   bash scripts/production/deploy_stack.sh --tag v1.0.0-abc1234
#
# Required environment variables (set via GitHub Actions secrets — environment: production):
#   PROD_HOST         VPS hostname or IP
#   PROD_USER         SSH user (must have docker access — typically 'deploy')
#   PROD_REPO_PATH    Repo checkout path on VPS  (default: /opt/7d-platform)
#   IMAGE_REGISTRY    Registry prefix             (default: 7dsolutions)
#
# Optional:
#   PROD_SSH_PORT     SSH port                    (default: 22)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
MANIFEST_FILE=""
TAG=""
DRY_RUN=false
SKIP_SMOKE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --manifest)   MANIFEST_FILE="$2"; shift 2 ;;
        --tag)        TAG="$2";           shift 2 ;;
        --dry-run)    DRY_RUN=true;       shift   ;;
        --skip-smoke) SKIP_SMOKE=true;    shift   ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# Default manifest path
if [[ -z "$MANIFEST_FILE" ]]; then
    MANIFEST_FILE="${REPO_ROOT}/deploy/production/MODULE-MANIFEST.md"
fi

# Refuse to deploy 'latest' — immutable tags only.
if [[ -n "$TAG" && ( "$TAG" == "latest" || "$TAG" == *":latest" ) ]]; then
    echo "ERROR: Refusing to deploy 'latest' tag. Specify an immutable tag." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Resolve image tags from manifest (unless --tag was given directly)
# ---------------------------------------------------------------------------
if [[ -z "$TAG" ]]; then
    if [[ ! -f "$MANIFEST_FILE" ]]; then
        echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
        echo "       Create deploy/production/MODULE-MANIFEST.md first." >&2
        exit 1
    fi

    # Extract unique non-pending image tags from manifest table.
    # Manifest columns: | Description | `canonical` | version | sha | `full-image-tag` | notes |
    RESOLVED_TAGS=()
    while IFS='|' read -r _desc canonical _ver sha_field full_tag _notes; do
        canonical="$(echo "$canonical" | tr -d '[:space:]`')"
        sha_field="$(echo "$sha_field" | tr -d '[:space:]')"
        full_tag="$(echo "$full_tag"   | tr -d '[:space:]`')"
        [[ -z "$canonical" || -z "$full_tag" ]] && continue
        # Skip pending entries
        if [[ "$sha_field" == "—" || "$full_tag" == *"{sha}"* ]]; then
            echo "[deploy_stack] WARN: Skipping pending entry: ${canonical}" >&2
            continue
        fi
        RESOLVED_TAGS+=("${canonical}|${full_tag}")
    done < <(grep '^|' "$MANIFEST_FILE" \
               | grep -v '^| Service\|^|---\|^| ---')

    if [[ ${#RESOLVED_TAGS[@]} -eq 0 ]]; then
        echo "ERROR: No resolved (non-pending) image tags found in ${MANIFEST_FILE}." >&2
        echo "       Update the manifest with real tags before deploying." >&2
        exit 1
    fi

    USE_MANIFEST=true
else
    USE_MANIFEST=false
fi

# ---------------------------------------------------------------------------
# Connection config
# ---------------------------------------------------------------------------
PROD_HOST="${PROD_HOST:?ERROR: PROD_HOST must be set}"
PROD_USER="${PROD_USER:-deploy}"
PROD_REPO_PATH="${PROD_REPO_PATH:-/opt/7d-platform}"
PROD_SSH_PORT="${PROD_SSH_PORT:-22}"
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${PROD_SSH_PORT}"
SSH_TARGET="${PROD_USER}@${PROD_HOST}"

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[deploy_stack:prod] $*"; }

run_remote() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] ssh ${SSH_TARGET}: $*"
    else
        # shellcheck disable=SC2029
        ssh $SSH_OPTS "$SSH_TARGET" "cd ${PROD_REPO_PATH} && $*"
    fi
}

run_remote_raw() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] ssh ${SSH_TARGET}: $*"
    else
        # shellcheck disable=SC2029
        ssh $SSH_OPTS "$SSH_TARGET" "$*"
    fi
}

# ---------------------------------------------------------------------------
# Services in deploy order (manifest-mode uses per-service tags; tag-mode uses global tag)
# Must stay in sync with staging/build_images.sh / staging/push_images.sh
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

if $USE_MANIFEST; then
    log "Manifest: ${MANIFEST_FILE}"
    log "Services with resolved tags:"
    for entry in "${RESOLVED_TAGS[@]}"; do
        svc="${entry%%|*}"
        img="${entry##*|}"
        log "  ${svc} → ${img}"
    done
else
    log "Tag (direct): ${TAG}"
    log "WARNING: --tag bypasses the manifest. Use --manifest for normal production deploys."
fi

# ---------------------------------------------------------------------------
# Step 1: Pull images from registry
# ---------------------------------------------------------------------------
banner "Pulling images"
if $USE_MANIFEST; then
    for entry in "${RESOLVED_TAGS[@]}"; do
        full_image="${entry##*|}"
        log "  docker pull ${full_image}"
        run_remote "docker pull ${full_image}"
    done
else
    for svc in "${SERVICES[@]}"; do
        full_image="${REGISTRY}/${svc}:${TAG}"
        log "  docker pull ${full_image}"
        run_remote "docker pull ${full_image}"
    done
fi

# ---------------------------------------------------------------------------
# Step 2: Write IMAGE_TAG to VPS .deploy.env
# For manifest-mode, per-service image references are embedded in Compose overrides.
# For tag-mode, a single IMAGE_TAG drives all services.
# ---------------------------------------------------------------------------
banner "Configuring IMAGE_TAG on VPS"
DEPLOY_ENV_PATH="${PROD_REPO_PATH}/.deploy.env"
if $USE_MANIFEST; then
    # Write per-service image env vars (e.g. CONTROL_PLANE_IMAGE=7dsolutions/control-plane:1.0.0-abc1234)
    {
        echo "REGISTRY=${REGISTRY}"
        for entry in "${RESOLVED_TAGS[@]}"; do
            svc="${entry%%|*}"
            img="${entry##*|}"
            # Normalise service name to env var (hyphens → underscores, uppercase)
            var="$(echo "${svc}" | tr '[:lower:]-' '[:upper:]_')_IMAGE"
            echo "${var}=${img}"
        done
    } | run_remote_raw "cat > ${DEPLOY_ENV_PATH}"
    log "Wrote per-service image env vars to .deploy.env"
else
    run_remote_raw "echo 'IMAGE_TAG=${TAG}' > ${DEPLOY_ENV_PATH} && echo 'REGISTRY=${REGISTRY}' >> ${DEPLOY_ENV_PATH}"
    log "Wrote IMAGE_TAG=${TAG} to .deploy.env"
fi

# ---------------------------------------------------------------------------
# Step 2b: Detect secrets overlay
# ---------------------------------------------------------------------------
PROD_OVERLAY=""
if ! $DRY_RUN; then
    HAS_SECRETS="$(ssh $SSH_OPTS "$SSH_TARGET" \
        "test -d /etc/7d/production/secrets && echo yes || echo no")"
    if [[ "$HAS_SECRETS" == "yes" ]]; then
        PROD_OVERLAY="-f docker-compose.production.yml"
        log "Docker secrets overlay: ENABLED"
    else
        log "Docker secrets overlay: DISABLED (no /etc/7d/production/secrets/)"
        log "  Falling back to env-var injection. See docs/SECRETS.md to migrate."
    fi
else
    PROD_OVERLAY="-f docker-compose.production.yml"
fi

# ---------------------------------------------------------------------------
# Step 3: Apply images — restart stacks without rebuild
# ---------------------------------------------------------------------------
banner "Restarting stacks"

# Data stack (NATS + Postgres) — --no-recreate to avoid losing in-flight data.
run_remote "docker compose -f docker-compose.data.yml ${PROD_OVERLAY} up -d --no-recreate"

if $USE_MANIFEST; then
    # Platform stack
    run_remote "docker compose -f docker-compose.platform.yml ${PROD_OVERLAY} up -d --no-build --pull never"
    # Backend modules
    run_remote "docker compose ${PROD_OVERLAY} up -d --no-build --pull never"
    # Frontend
    run_remote "docker compose -f docker-compose.frontend.yml up -d --no-build --pull never"
else
    run_remote "IMAGE_TAG=${TAG} docker compose -f docker-compose.platform.yml ${PROD_OVERLAY} up -d --no-build --pull never"
    run_remote "IMAGE_TAG=${TAG} docker compose ${PROD_OVERLAY} up -d --no-build --pull never"
    run_remote "IMAGE_TAG=${TAG} docker compose -f docker-compose.frontend.yml up -d --no-build --pull never"
fi

# ---------------------------------------------------------------------------
# Step 4: Record deployment
# ---------------------------------------------------------------------------
DEPLOY_LOG="${PROD_REPO_PATH}/.production-deployments"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if $USE_MANIFEST; then
    RECORD="${TIMESTAMP} manifest=${MANIFEST_FILE} registry=${REGISTRY}"
else
    RECORD="${TIMESTAMP} tag=${TAG} registry=${REGISTRY} source=direct"
fi
run_remote_raw "echo '${RECORD}' >> ${DEPLOY_LOG} && tail -20 ${DEPLOY_LOG}"
log "Deployment recorded: ${RECORD}"

# ---------------------------------------------------------------------------
# Step 5: Show running containers
# ---------------------------------------------------------------------------
banner "Running containers"
run_remote "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}'"

# ---------------------------------------------------------------------------
# Step 6: Smoke checks (optional)
# Production ports are firewalled; run health checks from inside the VPS via SSH.
# ---------------------------------------------------------------------------
if $SKIP_SMOKE; then
    log "Smoke checks skipped (--skip-smoke)"
else
    banner "Smoke checks (via SSH, localhost)"
    log "Waiting 15s for services to become healthy..."
    if ! $DRY_RUN; then sleep 15; fi

    FAILED=0

    declare -A HEALTH_ENDPOINTS=(
        ["auth"]="http://localhost:8080/api/health"
        ["control-plane"]="http://localhost:8091/api/ready"
        ["ar"]="http://localhost:8086/api/health"
        ["payments"]="http://localhost:8088/api/health"
        ["ttp"]="http://localhost:8100/api/health"
        ["tcp-ui"]="http://localhost:3000/login"
    )

    for svc in "${!HEALTH_ENDPOINTS[@]}"; do
        url="${HEALTH_ENDPOINTS[$svc]}"
        if $DRY_RUN; then
            echo "  [DRY-RUN] curl $url (from VPS)"
        else
            # Run curl from inside the VPS where service ports are accessible
            status="$(ssh $SSH_OPTS "$SSH_TARGET" \
                "curl -s -o /dev/null -w '%{http_code}' --max-time 10 '${url}' 2>/dev/null || echo 000")"
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
log "Production deploy complete."
if $USE_MANIFEST; then
    log "Manifest: ${MANIFEST_FILE}"
else
    log "Tag: ${TAG}"
fi
