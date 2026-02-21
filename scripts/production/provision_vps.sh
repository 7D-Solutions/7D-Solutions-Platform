#!/usr/bin/env bash
# provision_vps.sh — Guided interactive runbook for provisioning a production VPS.
#
# This script is a runbook, not fully automated. Steps that require a provider
# console are marked [MANUAL]. Automated steps are executed directly.
#
# Provider-agnostic: works with Hetzner, DigitalOcean, Linode, Vultr, etc.
# Recommended spec: 4 vCPU / 16 GB RAM / 80 GB SSD / Ubuntu 24.04 LTS
#
# Usage:
#   cp scripts/production/env.example scripts/production/.env.production
#   # Edit .env.production and populate all values
#   bash scripts/production/provision_vps.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ---------------------------------------------------------------------------
# Load environment
# ---------------------------------------------------------------------------
ENV_FILE="$REPO_ROOT/scripts/production/.env.production"
if [ ! -f "$ENV_FILE" ]; then
    echo "ERROR: $ENV_FILE not found." >&2
    echo "Run: cp scripts/production/env.example scripts/production/.env.production" >&2
    echo "Then populate all values." >&2
    exit 1
fi
# shellcheck disable=SC1090
source "$ENV_FILE"

: "${PROD_HOST:?PROD_HOST must be set in .env.production}"
: "${PROD_INITIAL_USER:?PROD_INITIAL_USER must be set in .env.production}"
: "${PROD_DEPLOY_USER:?PROD_DEPLOY_USER must be set in .env.production}"
: "${PROD_DEPLOY_KEY:?PROD_DEPLOY_KEY must be set in .env.production}"
: "${PROD_REPO_PATH:?PROD_REPO_PATH must be set in .env.production}"
SSH_PORT="${PROD_SSH_PORT:-22}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
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

ssh_initial() {
    ssh -o StrictHostKeyChecking=accept-new \
        -p "${SSH_PORT}" \
        "${PROD_INITIAL_USER}@${PROD_HOST}" "$@"
}

ssh_deploy() {
    ssh -o StrictHostKeyChecking=accept-new \
        -p "${SSH_PORT}" \
        "${PROD_DEPLOY_USER}@${PROD_HOST}" "$@"
}

# ---------------------------------------------------------------------------
# STEP 1: Create the VPS instance [MANUAL]
# ---------------------------------------------------------------------------
banner "STEP 1: Create VPS instance [MANUAL]"
echo "  Recommended spec: 4 vCPU / 16 GB RAM / 80 GB SSD"
echo "  OS: Ubuntu 24.04 LTS"
echo "  Region: choose closest to your users"
echo "  Networking:"
echo "    - Add your SSH public key during VPS creation"
echo "    - Block all inbound ports at the provider firewall (we harden with UFW)"
echo ""
echo "  After creating the instance, set PROD_HOST in .env.production."
echo "  Current PROD_HOST: ${PROD_HOST}"
pause "VPS created and PROD_HOST set in .env.production"

# ---------------------------------------------------------------------------
# STEP 2: Verify initial SSH access
# ---------------------------------------------------------------------------
banner "STEP 2: Verify initial SSH access"
echo "  Testing SSH as ${PROD_INITIAL_USER}@${PROD_HOST} (port ${SSH_PORT}) ..."
ssh_initial "echo '✓ Initial SSH access confirmed as $(whoami) on $(hostname)'"

# ---------------------------------------------------------------------------
# STEP 3: Create deploy user and install SSH key
# ---------------------------------------------------------------------------
banner "STEP 3: Create deploy user: ${PROD_DEPLOY_USER}"
echo "  Creating ${PROD_DEPLOY_USER} on ${PROD_HOST} (idempotent) ..."

if [ ! -f "${PROD_DEPLOY_KEY}" ]; then
    echo "ERROR: PROD_DEPLOY_KEY not found: ${PROD_DEPLOY_KEY}" >&2
    echo "Generate a key pair: ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519_prod" >&2
    exit 1
fi

PUBKEY="$(cat "${PROD_DEPLOY_KEY}")"

ssh_initial "bash -s" << REMOTE_EOF
set -euo pipefail
if id '${PROD_DEPLOY_USER}' &>/dev/null; then
    echo "    [skip] User ${PROD_DEPLOY_USER} already exists"
else
    useradd -m -s /bin/bash -G sudo '${PROD_DEPLOY_USER}'
    echo '${PROD_DEPLOY_USER} ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/${PROD_DEPLOY_USER}
    chmod 440 /etc/sudoers.d/${PROD_DEPLOY_USER}
    echo "    [ok]   User ${PROD_DEPLOY_USER} created"
fi

# Install SSH public key for deploy user
DEPLOY_HOME=\$(getent passwd '${PROD_DEPLOY_USER}' | cut -d: -f6)
install -d -m 700 -o '${PROD_DEPLOY_USER}' -g '${PROD_DEPLOY_USER}' "\${DEPLOY_HOME}/.ssh"
PUBKEY='${PUBKEY}'
if grep -qF "\${PUBKEY}" "\${DEPLOY_HOME}/.ssh/authorized_keys" 2>/dev/null; then
    echo "    [skip] SSH key already in authorized_keys"
else
    echo "\${PUBKEY}" >> "\${DEPLOY_HOME}/.ssh/authorized_keys"
    chown '${PROD_DEPLOY_USER}:${PROD_DEPLOY_USER}' "\${DEPLOY_HOME}/.ssh/authorized_keys"
    chmod 600 "\${DEPLOY_HOME}/.ssh/authorized_keys"
    echo "    [ok]   SSH key installed for ${PROD_DEPLOY_USER}"
fi
REMOTE_EOF

echo ""
echo "  Verifying deploy user SSH access ..."
ssh_deploy "echo '✓ Deploy user SSH access confirmed as $(whoami) on $(hostname)'"

# ---------------------------------------------------------------------------
# STEP 4: Run ssh_bootstrap.sh on the VPS
# ---------------------------------------------------------------------------
banner "STEP 4: Harden host and install Docker"
echo "  Running ssh_bootstrap.sh on ${PROD_HOST} as ${PROD_DEPLOY_USER} (sudo) ..."
echo "  This installs: UFW, fail2ban, SSH hardening, unattended-upgrades, auditd, Docker"
echo ""
PROD_SSH_PORT="${SSH_PORT}" \
    ssh_deploy "sudo bash -s" < "$REPO_ROOT/scripts/production/ssh_bootstrap.sh"

# ---------------------------------------------------------------------------
# STEP 5: Clone repository on VPS [MANUAL]
# ---------------------------------------------------------------------------
banner "STEP 5: Clone repository on VPS [MANUAL]"
echo "  SSH into the VPS and clone the repo:"
echo ""
echo "    ssh -p ${SSH_PORT} ${PROD_DEPLOY_USER}@${PROD_HOST}"
echo "    git clone git@github.com:7D-Solutions/7D-Solutions-Platform.git ${PROD_REPO_PATH}"
echo "    exit"
echo ""
echo "  If you are using HTTPS instead of SSH:"
echo "    git clone https://github.com/7D-Solutions/7D-Solutions-Platform.git ${PROD_REPO_PATH}"
pause "Repository is present at ${PROD_REPO_PATH} on the VPS"

# ---------------------------------------------------------------------------
# STEP 6: Upload secrets and deploy [MANUAL — see DEPLOYMENT-PRODUCTION.md]
# ---------------------------------------------------------------------------
banner "STEP 6: Secrets and first deploy"
echo "  Secrets (DB passwords, JWT keys, etc.) are managed in the next phase."
echo "  See: docs/DEPLOYMENT-PRODUCTION.md → 'First Production Deploy'"
echo ""
echo "  When secrets are ready:"
echo "    bash scripts/staging/deploy_stack.sh --tag <version-tag> (pointed at PROD_HOST)"
echo "  Or run promote.yml against the production environment in GitHub Actions."
echo ""

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
banner "PROVISIONING COMPLETE"
echo "  Host:       ${PROD_HOST}"
echo "  Deploy user:${PROD_DEPLOY_USER}"
echo "  Repo path:  ${PROD_REPO_PATH}"
echo "  SSH port:   ${SSH_PORT}"
echo ""
echo "  Security posture:"
echo "    - Password auth: DISABLED"
echo "    - Root login via SSH: DISABLED"
echo "    - UFW: active (SSH/${SSH_PORT}, 80, 443)"
echo "    - fail2ban: active"
echo "    - Unattended security upgrades: enabled"
echo "    - auditd: active"
echo ""
echo "  Next bead: bd-1itw — Production env contract and secrets layout"
