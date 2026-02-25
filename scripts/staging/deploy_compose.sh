#!/usr/bin/env bash
# deploy_compose.sh — Pull latest code and bring up the full Docker Compose stack
# on the staging VPS via SSH.
#
# Usage:
#   bash scripts/staging/deploy_compose.sh              # full deploy
#   bash scripts/staging/deploy_compose.sh --smoke-only # skip deploy, run smoke checks only
#
# Prerequisites:
#   - scripts/staging/.env.staging is populated (run export_env.sh first)
#   - VPS has been bootstrapped (run ssh_bootstrap.sh first)
#   - Repo is checked out at STAGING_REPO_PATH on the VPS

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SMOKE_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --smoke-only) SMOKE_ONLY=true ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

# Load local env to get connection details
ENV_FILE="$REPO_ROOT/scripts/staging/.env.staging"
if [ ! -f "$ENV_FILE" ]; then
    echo "ERROR: scripts/staging/.env.staging not found." >&2
    echo "Run: cp scripts/staging/env.example scripts/staging/.env.staging" >&2
    exit 1
fi
# shellcheck disable=SC1090
source "$REPO_ROOT/scripts/staging/export_env.sh" "$ENV_FILE"

SSH_TARGET="${STAGING_USER}@${STAGING_HOST}"
REPO_PATH="${STAGING_REPO_PATH}"

banner() { echo ""; echo "=== $1 ==="; }

# -------------------------------------------------------
# Remote deploy function — runs commands on VPS
# -------------------------------------------------------
remote() {
    ssh "$SSH_TARGET" "cd $REPO_PATH && $*"
}

# -------------------------------------------------------
# Smoke checks — curl /api/ready on all services
# -------------------------------------------------------
smoke_check() {
    local host="$STAGING_HOST"
    local failed=0

    banner "Smoke checks"

    declare -A ENDPOINTS=(
        ["auth-lb"]="http://${host}:8080/api/health"
        ["control-plane"]="http://${host}:8091/api/ready"
        ["ar"]="http://${host}:8086/api/health"
        ["subscriptions"]="http://${host}:8087/api/health"
        ["payments"]="http://${host}:8088/api/health"
        ["notifications"]="http://${host}:8089/api/health"
        ["gl"]="http://${host}:8090/api/health"
        ["inventory"]="http://${host}:8092/api/health"
        ["ap"]="http://${host}:8093/api/health"
        ["treasury"]="http://${host}:8094/api/health"
        ["fixed-assets"]="http://${host}:8104/api/health"
        ["consolidation"]="http://${host}:8105/api/health"
        ["timekeeping"]="http://${host}:8097/api/health"
        ["party"]="http://${host}:8098/api/health"
        ["integrations"]="http://${host}:8099/api/health"
        ["ttp"]="http://${host}:8100/api/health"
        ["tcp-ui"]="http://${host}:3000/login"
    )

    for svc in "${!ENDPOINTS[@]}"; do
        local url="${ENDPOINTS[$svc]}"
        local status
        status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$url" 2>/dev/null || echo "000")
        if [[ "$status" == "200" ]]; then
            echo "  ✓ ${svc} ($status)"
        else
            echo "  ✗ ${svc} — HTTP $status ($url)"
            failed=$((failed + 1))
        fi
    done

    echo ""
    if [ "$failed" -eq 0 ]; then
        echo "All smoke checks passed."
    else
        echo "$failed service(s) failed smoke checks."
        return 1
    fi
}

# -------------------------------------------------------
# Main deploy flow
# -------------------------------------------------------
if [ "$SMOKE_ONLY" = true ]; then
    smoke_check
    exit $?
fi

banner "Deploying to ${STAGING_HOST}"

# 1. Pull latest code
banner "Git pull"
remote "git pull --ff-only"

# 2. Copy env file to VPS (re-upload in case it changed)
banner "Uploading env file"
scp "$ENV_FILE" "${SSH_TARGET}:${REPO_PATH}/.env"
echo "  ✓ .env uploaded"

# 3. Build and start data stack (NATS + Postgres)
banner "Data stack (NATS + Postgres)"
remote "docker compose -f docker-compose.data.yml pull --quiet 2>/dev/null || true"
remote "docker compose -f docker-compose.data.yml up -d --build"
echo "  Waiting 15s for databases to initialise ..."
sleep 15

# 4. Start platform stack (auth, control-plane)
banner "Platform stack (auth, control-plane)"
remote "docker compose -f docker-compose.platform.yml up -d --build"

# 5. Start backend modules stack
banner "Backend modules stack"
remote "docker compose up -d --build"

# 6. Start frontend stack
banner "Frontend stack (TCP UI)"
remote "docker compose -f docker-compose.frontend.yml up -d --build"

# 7. Show running containers
banner "Running containers"
remote "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'"

# 8. Wait for services and run smoke checks
echo ""
echo "Waiting 30s for services to become healthy ..."
sleep 30
smoke_check
