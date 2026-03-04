#!/usr/bin/env bash
# secrets_init.sh — Generate all production secret files from scratch.
#
# Creates /etc/7d/production/secrets/ with one file per secret value.
# Each file is root:root 0600. Generates random passwords for databases
# and NATS, and prompts for JWT keys (or generates them).
#
# Safe to re-run: skips existing files unless --force is given.
#
# Usage (on VPS, as root or sudo):
#   sudo bash scripts/production/secrets_init.sh
#   sudo bash scripts/production/secrets_init.sh --force    # overwrite existing
#   sudo bash scripts/production/secrets_init.sh --dry-run  # show what would be created
#
# After running, validate with:
#   sudo bash scripts/production/secrets_check.sh --format=dir

set -euo pipefail

SECRETS_DIR="/etc/7d/production/secrets"
FORCE=false
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --force)   FORCE=true;   shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# Must be root
if [ "$(id -u)" -ne 0 ] && ! $DRY_RUN; then
    echo "ERROR: Must run as root (use sudo)." >&2
    exit 1
fi

gen_password() {
    openssl rand -base64 32 | tr -d '\n'
}

write_secret() {
    local name="$1"
    local value="$2"
    local path="${SECRETS_DIR}/${name}"

    if [ -f "$path" ] && ! $FORCE; then
        echo "  SKIP  ${name} (exists — use --force to overwrite)"
        return
    fi

    if $DRY_RUN; then
        echo "  WOULD CREATE  ${name} (${#value} bytes)"
        return
    fi

    printf '%s' "$value" > "$path"
    chmod 0600 "$path"
    chown root:root "$path"
    echo "  OK    ${name}"
}

echo "=== 7D Solutions — Production Secrets Initialisation ==="
echo "Secrets directory: ${SECRETS_DIR}"
echo ""

if ! $DRY_RUN; then
    mkdir -p "$SECRETS_DIR"
    chmod 0700 "$SECRETS_DIR"
    chown root:root "$SECRETS_DIR"
fi

# ---------------------------------------------------------------------------
# NATS auth token
# ---------------------------------------------------------------------------
echo "--- NATS ---"
NATS_TOKEN="$(gen_password)"
write_secret "nats_auth_token" "$NATS_TOKEN"
write_secret "nats_url" "nats://platform:${NATS_TOKEN}@7d-nats:4222"

# ---------------------------------------------------------------------------
# JWT keys
# ---------------------------------------------------------------------------
echo "--- JWT ---"
JWT_TMPDIR="$(mktemp -d)"
trap 'rm -rf "$JWT_TMPDIR"' EXIT

openssl genpkey -algorithm ed25519 -out "${JWT_TMPDIR}/private.pem" 2>/dev/null
openssl pkey -in "${JWT_TMPDIR}/private.pem" -pubout -out "${JWT_TMPDIR}/public.pem" 2>/dev/null

JWT_PRIVATE="$(cat "${JWT_TMPDIR}/private.pem")"
JWT_PUBLIC="$(cat "${JWT_TMPDIR}/public.pem")"

write_secret "jwt_private_key_pem" "$JWT_PRIVATE"
write_secret "jwt_public_key_pem" "$JWT_PUBLIC"

# BFF session secret
write_secret "jwt_secret" "$(openssl rand -hex 32)"

# ---------------------------------------------------------------------------
# Database passwords + full DATABASE_URL secrets
# ---------------------------------------------------------------------------
echo "--- Database passwords ---"

# Service name | DB user | DB name | Postgres host | Secret prefix
declare -a DB_SERVICES=(
    "auth|auth_user|auth_db|7d-auth-postgres|auth"
    "ar|ar_user|ar_db|7d-ar-postgres|ar"
    "subscriptions|subscriptions_user|subscriptions_db|7d-subscriptions-postgres|subscriptions"
    "payments|payments_user|payments_db|7d-payments-postgres|payments"
    "notifications|notifications_user|notifications_db|7d-notifications-postgres|notifications"
    "gl|gl_user|gl_db|7d-gl-postgres|gl"
    "inventory|inventory_user|inventory_db|7d-inventory-postgres|inventory"
    "ap|ap_user|ap_db|7d-ap-postgres|ap"
    "treasury|treasury_user|treasury_db|7d-treasury-postgres|treasury"
    "fixed_assets|fixed_assets_user|fixed_assets_db|7d-fixed-assets-postgres|fixed_assets"
    "consolidation|consolidation_user|consolidation_db|7d-consolidation-postgres|consolidation"
    "timekeeping|timekeeping_user|timekeeping_db|7d-timekeeping-postgres|timekeeping"
    "party|party_user|party_db|7d-party-postgres|party"
    "integrations|integrations_user|integrations_db|7d-integrations-postgres|integrations"
    "ttp|ttp_user|ttp_db|7d-ttp-postgres|ttp"
    "pdf_editor|pdf_editor_user|pdf_editor_db|7d-pdf-editor-postgres|pdf_editor"
    "maintenance|maintenance_user|maintenance_db|7d-maintenance-postgres|maintenance"
    "shipping_receiving|shipping_receiving_user|shipping_receiving_db|7d-shipping-receiving-postgres|shipping_receiving"
    "tenant_registry|tenant_registry_user|tenant_registry_db|7d-tenant-registry-postgres|tenant_registry"
    "projections|projections_user|projections_db|7d-projections-postgres|projections"
    "audit|audit_user|audit_db|7d-audit-postgres|audit"
    "numbering|numbering_user|numbering_db|7d-numbering-postgres|numbering"
    "doc_mgmt|doc_mgmt_user|doc_mgmt_db|7d-doc-mgmt-postgres|doc_mgmt"
    "workflow|workflow_user|workflow_db|7d-workflow-postgres|workflow"
    "wc|wc_user|workforce_competence_db|7d-workforce-competence-postgres|wc"
)

for entry in "${DB_SERVICES[@]}"; do
    IFS='|' read -r _svc db_user db_name db_host prefix <<< "$entry"
    pw="$(gen_password)"
    write_secret "${prefix}_postgres_password" "$pw"
    write_secret "${prefix}_database_url" "postgres://${db_user}:${pw}@${db_host}:5432/${db_name}"
done

# Control plane needs AR database URL too
echo "--- Control plane AR database URL ---"
AR_PW_FILE="${SECRETS_DIR}/ar_postgres_password"
if [ -f "$AR_PW_FILE" ]; then
    AR_PW="$(cat "$AR_PW_FILE")"
else
    AR_PW="$(gen_password)"
fi
write_secret "control_plane_ar_database_url" "postgres://ar_user:${AR_PW}@7d-ar-postgres:5432/ar_db"

# ---------------------------------------------------------------------------
# Application secrets
# ---------------------------------------------------------------------------
echo "--- Application secrets ---"
write_secret "tilled_webhook_secret" "$(gen_password)"
write_secret "seed_admin_password" "$(gen_password)"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if $DRY_RUN; then
    echo "Dry run complete. No files written."
else
    FILE_COUNT="$(find "$SECRETS_DIR" -type f | wc -l | tr -d ' ')"
    echo "Done. ${FILE_COUNT} secret files in ${SECRETS_DIR}"
    echo ""
    echo "Next steps:"
    echo "  1. Validate:  sudo bash scripts/production/secrets_check.sh --format=dir"
    echo "  2. Deploy:    see docs/SECRETS.md"
fi
