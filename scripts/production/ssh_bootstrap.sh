#!/usr/bin/env bash
# ssh_bootstrap.sh — Harden and provision a fresh Ubuntu 24.04 VPS for production.
#
# Run via SSH from the local machine (as the initial root/sudo user):
#   ssh root@host 'bash -s' < scripts/production/ssh_bootstrap.sh
#
# Or directly on the VPS:
#   sudo bash ssh_bootstrap.sh
#
# Idempotent: safe to run multiple times. Skips steps already done.
#
# Differences from scripts/staging/ssh_bootstrap.sh:
#   - SSH hardening drop-in: no password auth, no root login, tighter timeouts
#   - UFW firewall: deny-in by default, open SSH / 80 / 443 only
#   - fail2ban: SSH brute-force protection with auto-ban
#   - Unattended security upgrades (security patches only, no auto-reboot)
#   - auditd: auth/config change monitoring with persistent rules
#   - Docker + network + volumes identical to staging (compositional parity)

set -euo pipefail

# Optional: override SSH port via env variable (default 22).
SSH_PORT="${PROD_SSH_PORT:-22}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
banner() {
    echo ""
    echo "============================================"
    echo "  $1"
    echo "============================================"
}

already() { echo "    [skip] $1 — already done"; }
done_msg() { echo "    [ok]   $1"; }

echo "=== 7D Platform Production VPS Bootstrap ==="
echo "Host: $(hostname)"
echo "Date: $(date -u)"
echo "SSH port that will be opened in UFW: ${SSH_PORT}"
echo ""

# ---------------------------------------------------------------------------
# 1. System packages
# ---------------------------------------------------------------------------
banner "1. System packages"
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
    unzip \
    ufw \
    fail2ban \
    unattended-upgrades \
    apt-listchanges \
    auditd \
    audispd-plugins

# ---------------------------------------------------------------------------
# 2. SSH hardening (drop-in config — does not overwrite distro defaults)
# ---------------------------------------------------------------------------
banner "2. SSH hardening"
SSHD_DROP_IN="/etc/ssh/sshd_config.d/99-hardened.conf"
if [ -f "$SSHD_DROP_IN" ]; then
    already "SSH drop-in $SSHD_DROP_IN"
else
    cat > "$SSHD_DROP_IN" << 'EOF'
# 7D Platform production SSH hardening — managed by ssh_bootstrap.sh
PasswordAuthentication no
PermitEmptyPasswords no
PermitRootLogin no
PubkeyAuthentication yes
MaxAuthTries 3
LoginGraceTime 30s
X11Forwarding no
AllowAgentForwarding no
AllowTcpForwarding no
ClientAliveInterval 300
ClientAliveCountMax 2
LogLevel VERBOSE
EOF
    systemctl reload sshd
    done_msg "SSH drop-in written and sshd reloaded"
fi

# ---------------------------------------------------------------------------
# 3. UFW firewall
# ---------------------------------------------------------------------------
banner "3. UFW firewall"
if ufw status | grep -q "Status: active"; then
    already "UFW already active — skipping rule setup"
else
    ufw --force reset
    ufw default deny incoming
    ufw default allow outgoing
    ufw allow "${SSH_PORT}/tcp" comment '7D production SSH'
    ufw allow 80/tcp  comment '7D production HTTP (nginx redirect)'
    ufw allow 443/tcp comment '7D production HTTPS (nginx TLS)'
    # Service ports (3000, 8080-8100) are NOT opened externally.
    # nginx reverse-proxies all traffic at 80/443.
    ufw --force enable
    done_msg "UFW enabled: deny-in, allow SSH(${SSH_PORT})/80/443"
fi

# ---------------------------------------------------------------------------
# 4. fail2ban
# ---------------------------------------------------------------------------
banner "4. fail2ban"
FAIL2BAN_JAIL="/etc/fail2ban/jail.local"
if [ -f "$FAIL2BAN_JAIL" ]; then
    already "fail2ban jail.local"
else
    cat > "$FAIL2BAN_JAIL" << EOF
[DEFAULT]
bantime  = 3600
findtime = 600
maxretry = 5

[sshd]
enabled  = true
port     = ${SSH_PORT}
logpath  = %(sshd_log)s
backend  = %(sshd_backend)s
maxretry = 3
EOF
    systemctl enable fail2ban
    systemctl restart fail2ban
    done_msg "fail2ban configured and started"
fi

# ---------------------------------------------------------------------------
# 5. Unattended security upgrades
# ---------------------------------------------------------------------------
banner "5. Unattended upgrades"
UA_CONF="/etc/apt/apt.conf.d/50unattended-upgrades"
if grep -q "7D Platform" "$UA_CONF" 2>/dev/null; then
    already "unattended-upgrades already configured"
else
    cat > "$UA_CONF" << 'EOF'
// 7D Platform production — security updates only, no auto-reboot.
Unattended-Upgrade::Allowed-Origins {
    "${distro_id}:${distro_codename}-security";
};
Unattended-Upgrade::AutoFixInterruptedDpkg "true";
Unattended-Upgrade::MinimalSteps "true";
Unattended-Upgrade::InstallOnShutdown "false";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "false";
EOF
    cat > /etc/apt/apt.conf.d/20auto-upgrades << 'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
EOF
    done_msg "unattended-upgrades configured (security-only, no auto-reboot)"
fi

# ---------------------------------------------------------------------------
# 6. auditd — persistent auth and config monitoring rules
# ---------------------------------------------------------------------------
banner "6. auditd rules"
AUDIT_RULES="/etc/audit/rules.d/99-7d-hardened.rules"
if [ -f "$AUDIT_RULES" ]; then
    already "auditd rules $AUDIT_RULES"
else
    cat > "$AUDIT_RULES" << 'EOF'
# 7D Platform production audit rules — managed by ssh_bootstrap.sh
-w /var/log/auth.log        -p wa -k logins
-w /etc/passwd              -p wa -k passwd_changes
-w /etc/shadow              -p wa -k passwd_changes
-w /etc/group               -p wa -k group_changes
-w /etc/sudoers             -p wa -k sudoers_changes
-w /etc/ssh/sshd_config     -p wa -k sshd_config
-w /etc/ssh/sshd_config.d   -p wa -k sshd_config
-w /opt/7d-platform         -p wa -k repo_changes
EOF
    systemctl enable auditd
    systemctl restart auditd
    done_msg "auditd rules written and service restarted"
fi

# ---------------------------------------------------------------------------
# 7. Docker Engine
# ---------------------------------------------------------------------------
banner "7. Docker Engine"
if command -v docker &>/dev/null; then
    already "Docker $(docker --version)"
else
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
    done_msg "Docker installed: $(docker --version)"
fi

# ---------------------------------------------------------------------------
# 8. Add deploy user to docker group
# ---------------------------------------------------------------------------
banner "8. Docker group membership"
CURRENT_USER="${SUDO_USER:-$(whoami)}"
if [ "$CURRENT_USER" != "root" ]; then
    if groups "$CURRENT_USER" | grep -q docker; then
        already "$CURRENT_USER already in docker group"
    else
        usermod -aG docker "$CURRENT_USER"
        done_msg "Added $CURRENT_USER to docker group (log out/in to activate)"
    fi
else
    echo "    Running as root — no group change needed."
fi

# ---------------------------------------------------------------------------
# 9. Docker network (same as staging — compositional parity)
# ---------------------------------------------------------------------------
banner "9. Docker network"
if docker network inspect 7d-platform &>/dev/null; then
    already "Docker network 7d-platform"
else
    docker network create 7d-platform
    done_msg "Docker network 7d-platform created"
fi

# ---------------------------------------------------------------------------
# 10. Docker volumes (same as staging — compositional parity)
# ---------------------------------------------------------------------------
banner "10. Docker volumes"
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
    7d-maintenance-pgdata
    7d-pdf-editor-pgdata
    7d-shipping-receiving-pgdata
    7d-numbering-pgdata
    7d-doc-mgmt-pgdata
    7d-workflow-pgdata
    7d-workforce-competence-pgdata
    7d-bom-pgdata
    7d-production-pgdata
    7d-quality-inspection-pgdata
    7d-customer-portal-pgdata
    7d-reporting-pgdata
)

for vol in "${VOLUMES[@]}"; do
    if docker volume inspect "$vol" &>/dev/null; then
        echo "    $vol (exists)"
    else
        docker volume create "$vol"
        echo "    $vol (created)"
    fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Production bootstrap complete ==="
echo "Docker:          $(docker --version)"
echo "Docker Compose:  $(docker compose version)"
echo "UFW status:      $(ufw status | head -1)"
echo "fail2ban:        $(systemctl is-active fail2ban)"
echo "auditd:          $(systemctl is-active auditd)"
echo ""
echo "IMPORTANT: Verify SSH key-based login works as the deploy user"
echo "before closing the current root session."
echo ""
echo "Next step: run scripts/production/provision_vps.sh steps 5-6 from local machine."
