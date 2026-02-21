#!/usr/bin/env bash
# provision_vps.sh — Documented VPS provisioning checklist.
#
# This script is a runbook, not fully automated. It documents the exact steps
# to provision a fresh VPS for staging. Steps that require a provider console
# are marked [MANUAL]. Steps that can be scripted are automated here.
#
# Provider-agnostic: works with Hetzner, DigitalOcean, Linode, Vultr, etc.
# Recommended spec: 4 vCPU / 16 GB RAM / 80 GB SSD / Ubuntu 24.04 LTS
#
# Usage:
#   bash scripts/staging/provision_vps.sh
#   (run interactively — it will prompt before each manual step)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

source_env() {
    local env_file="$REPO_ROOT/scripts/staging/.env.staging"
    if [ ! -f "$env_file" ]; then
        echo "ERROR: scripts/staging/.env.staging not found." >&2
        echo "Run: cp scripts/staging/env.example scripts/staging/.env.staging" >&2
        echo "Then populate all values." >&2
        exit 1
    fi
    # shellcheck disable=SC1090
    source "$REPO_ROOT/scripts/staging/export_env.sh" "$env_file"
}

pause() {
    echo ""
    echo ">>> $1"
    echo "    Press ENTER when complete, or Ctrl-C to abort."
    read -r
}

banner() {
    echo ""
    echo "============================================"
    echo "  $1"
    echo "============================================"
}

source_env

banner "STEP 1: Create VPS instance [MANUAL]"
echo "  Recommended spec: 4 vCPU / 16 GB RAM / 80 GB SSD"
echo "  OS: Ubuntu 24.04 LTS"
echo "  Region: choose closest to your users"
echo "  Networking: enable firewall, allow only:"
echo "    - SSH (22) from your IP"
echo "    - HTTP (80) and HTTPS (443) from everywhere"
echo "    - Optionally: service ports (8080-8100) for internal access only"
echo ""
echo "  After creating the instance, set STAGING_HOST in .env.staging."
echo "  Current STAGING_HOST: ${STAGING_HOST}"
pause "VPS created and STAGING_HOST set in .env.staging"

banner "STEP 2: Configure SSH access"
echo "  Ensure your SSH public key is installed on the VPS."
echo "  Test: ssh ${STAGING_USER}@${STAGING_HOST} echo OK"
echo ""
ssh "${STAGING_USER}@${STAGING_HOST}" "echo '✓ SSH access confirmed'"

banner "STEP 3: Bootstrap Docker + dependencies on VPS"
echo "  Running ssh_bootstrap.sh on ${STAGING_HOST} ..."
ssh "${STAGING_USER}@${STAGING_HOST}" 'bash -s' < "$REPO_ROOT/scripts/staging/ssh_bootstrap.sh"

banner "STEP 4: Upload env file to VPS"
echo "  Copying .env.staging to VPS (${STAGING_REPO_PATH}/.env) ..."
ssh "${STAGING_USER}@${STAGING_HOST}" "mkdir -p ${STAGING_REPO_PATH}"
scp "$REPO_ROOT/scripts/staging/.env.staging" \
    "${STAGING_USER}@${STAGING_HOST}:${STAGING_REPO_PATH}/.env"
echo "  ✓ Env file uploaded"

banner "STEP 5: Clone repository on VPS [MANUAL if first time]"
echo "  If this is a fresh VPS, clone the repo:"
echo "    ssh ${STAGING_USER}@${STAGING_HOST}"
echo "    git clone <your-repo-url> ${STAGING_REPO_PATH}"
echo ""
echo "  If repo already exists, the deploy script will pull latest."
pause "Repository is present at ${STAGING_REPO_PATH} on the VPS"

banner "STEP 6: Run initial deploy"
echo "  Running deploy_compose.sh ..."
"$REPO_ROOT/scripts/staging/deploy_compose.sh"

banner "DONE"
echo "  Staging environment is running at http://${STAGING_HOST}"
echo "  TCP UI:       http://${STAGING_HOST}:3000"
echo "  Auth:         http://${STAGING_HOST}:8080"
echo "  Control Plane:http://${STAGING_HOST}:8091"
echo ""
echo "  Run smoke checks:"
echo "    bash scripts/staging/deploy_compose.sh --smoke-only"
