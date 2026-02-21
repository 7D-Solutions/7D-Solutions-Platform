#!/usr/bin/env bash
# ssh_bootstrap.sh — Install Docker + Docker Compose on a fresh Ubuntu 24.04 VPS.
#
# Run via SSH from the local machine:
#   ssh user@host 'bash -s' < scripts/staging/ssh_bootstrap.sh
#
# Or directly on the VPS:
#   bash ssh_bootstrap.sh
#
# Idempotent: safe to run multiple times. Skips steps that are already done.

set -euo pipefail

echo "=== 7D Platform VPS Bootstrap ==="
echo "Host: $(hostname)"
echo "Date: $(date -u)"
echo ""

# -------------------------------------------------------
# 1. System packages
# -------------------------------------------------------
echo "--- Installing system dependencies ---"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q \
    curl \
    git \
    ca-certificates \
    gnupg \
    lsb-release \
    jq \
    wget \
    unzip

# -------------------------------------------------------
# 2. Docker Engine
# -------------------------------------------------------
if command -v docker &>/dev/null; then
    echo "--- Docker already installed: $(docker --version) ---"
else
    echo "--- Installing Docker Engine ---"
    install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg \
        | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    chmod a+r /etc/apt/keyrings/docker.gpg

    echo \
        "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] \
        https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" \
        > /etc/apt/sources.list.d/docker.list

    apt-get update -q
    apt-get install -y -q \
        docker-ce \
        docker-ce-cli \
        containerd.io \
        docker-buildx-plugin \
        docker-compose-plugin

    systemctl enable docker
    systemctl start docker
    echo "--- Docker installed: $(docker --version) ---"
fi

# -------------------------------------------------------
# 3. Add deploy user to docker group (if not root)
# -------------------------------------------------------
CURRENT_USER="${SUDO_USER:-$(whoami)}"
if [ "$CURRENT_USER" != "root" ]; then
    if ! groups "$CURRENT_USER" | grep -q docker; then
        echo "--- Adding $CURRENT_USER to docker group ---"
        usermod -aG docker "$CURRENT_USER"
        echo "    NOTE: Log out and back in for group change to take effect."
    else
        echo "--- $CURRENT_USER already in docker group ---"
    fi
fi

# -------------------------------------------------------
# 4. Create Docker external network (idempotent)
# -------------------------------------------------------
if docker network inspect 7d-platform &>/dev/null; then
    echo "--- Docker network 7d-platform already exists ---"
else
    echo "--- Creating Docker network 7d-platform ---"
    docker network create 7d-platform
fi

# -------------------------------------------------------
# 5. Create Docker external volumes (idempotent)
# -------------------------------------------------------
VOLUMES=(
    7d-nats-data
    7d-auth-pgdata
    7d-ar-pgdata
    7d-subscriptions-pgdata
    7d-payments-pgdata
    7d-notifications-pgdata
    7d-gl-pgdata
    7d-projections-pgdata
    7d-audit-pgdata
    7d-tenant-registry-pgdata
    7d-inventory-pgdata
    7d-ap-pgdata
    7d-treasury-pgdata
    7d-fixed-assets-pgdata
    7d-consolidation-pgdata
    7d-timekeeping-pgdata
    7d-party-pgdata
    7d-integrations-pgdata
    7d-ttp-pgdata
)

echo "--- Creating Docker volumes ---"
for vol in "${VOLUMES[@]}"; do
    if docker volume inspect "$vol" &>/dev/null; then
        echo "    $vol (exists)"
    else
        docker volume create "$vol"
        echo "    $vol (created)"
    fi
done

# -------------------------------------------------------
# 6. Verify
# -------------------------------------------------------
echo ""
echo "=== Bootstrap complete ==="
echo "Docker: $(docker --version)"
echo "Docker Compose: $(docker compose version)"
echo "Network 7d-platform: $(docker network inspect 7d-platform --format '{{.Name}}')"
echo ""
echo "Next step: run deploy_compose.sh from your local machine."
