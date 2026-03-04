#!/usr/bin/env bash
# secrets_rotate.sh — Rotate a single production secret.
#
# Generates a new random value, updates the secret file, and updates any
# derived secrets (e.g. rotating a DB password also updates the DATABASE_URL).
#
# After rotating, redeploy the affected services:
#   docker compose -f docker-compose.services.yml -f docker-compose.production.yml up -d <service>
#
# Usage (on VPS, as root or sudo):
#   sudo bash scripts/production/secrets_rotate.sh nats
#   sudo bash scripts/production/secrets_rotate.sh db auth
#   sudo bash scripts/production/secrets_rotate.sh db all
#   sudo bash scripts/production/secrets_rotate.sh jwt
#   sudo bash scripts/production/secrets_rotate.sh --list
#
# IMPORTANT: Rotating a DB password only updates the secret files.
# You must also ALTER ROLE in Postgres to match the new password:
#   docker exec 7d-auth-postgres psql -U auth_user -d auth_db \
#     -c "ALTER ROLE auth_user WITH PASSWORD '<new-password>'"
# Then redeploy the service.

set -euo pipefail

SECRETS_DIR="/etc/7d/production/secrets"

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: Must run as root (use sudo)." >&2
    exit 1
fi

if [ ! -d "$SECRETS_DIR" ]; then
    echo "ERROR: Secrets directory not found: ${SECRETS_DIR}" >&2
    echo "Run secrets_init.sh first." >&2
    exit 1
fi

gen_password() {
    openssl rand -base64 32 | tr -d '\n'
}

write_secret() {
    local path="${SECRETS_DIR}/$1"
    local value="$2"
    printf '%s' "$value" > "$path"
    chmod 0600 "$path"
    chown root:root "$path"
    echo "  Updated: $1"
}

# DB service definitions: prefix|user|dbname|host
declare -A DB_MAP=(
    ["auth"]="auth_user|auth_db|7d-auth-postgres"
    ["ar"]="ar_user|ar_db|7d-ar-postgres"
    ["subscriptions"]="subscriptions_user|subscriptions_db|7d-subscriptions-postgres"
    ["payments"]="payments_user|payments_db|7d-payments-postgres"
    ["notifications"]="notifications_user|notifications_db|7d-notifications-postgres"
    ["gl"]="gl_user|gl_db|7d-gl-postgres"
    ["inventory"]="inventory_user|inventory_db|7d-inventory-postgres"
    ["ap"]="ap_user|ap_db|7d-ap-postgres"
    ["treasury"]="treasury_user|treasury_db|7d-treasury-postgres"
    ["fixed_assets"]="fixed_assets_user|fixed_assets_db|7d-fixed-assets-postgres"
    ["consolidation"]="consolidation_user|consolidation_db|7d-consolidation-postgres"
    ["timekeeping"]="timekeeping_user|timekeeping_db|7d-timekeeping-postgres"
    ["party"]="party_user|party_db|7d-party-postgres"
    ["integrations"]="integrations_user|integrations_db|7d-integrations-postgres"
    ["ttp"]="ttp_user|ttp_db|7d-ttp-postgres"
    ["pdf_editor"]="pdf_editor_user|pdf_editor_db|7d-pdf-editor-postgres"
    ["maintenance"]="maintenance_user|maintenance_db|7d-maintenance-postgres"
    ["shipping_receiving"]="shipping_receiving_user|shipping_receiving_db|7d-shipping-receiving-postgres"
    ["tenant_registry"]="tenant_registry_user|tenant_registry_db|7d-tenant-registry-postgres"
    ["projections"]="projections_user|projections_db|7d-projections-postgres"
    ["audit"]="audit_user|audit_db|7d-audit-postgres"
    ["numbering"]="numbering_user|numbering_db|7d-numbering-postgres"
    ["doc_mgmt"]="doc_mgmt_user|doc_mgmt_db|7d-doc-mgmt-postgres"
    ["workflow"]="workflow_user|workflow_db|7d-workflow-postgres"
    ["wc"]="wc_user|workforce_competence_db|7d-workforce-competence-postgres"
)

rotate_db() {
    local prefix="$1"
    if [[ -z "${DB_MAP[$prefix]+x}" ]]; then
        echo "ERROR: Unknown DB service: ${prefix}" >&2
        echo "Valid services: ${!DB_MAP[*]}" >&2
        exit 1
    fi

    IFS='|' read -r db_user db_name db_host <<< "${DB_MAP[$prefix]}"
    local new_pw
    new_pw="$(gen_password)"

    echo "Rotating DB password for: ${prefix}"
    write_secret "${prefix}_postgres_password" "$new_pw"
    write_secret "${prefix}_database_url" "postgres://${db_user}:${new_pw}@${db_host}:5432/${db_name}"

    # If this is the AR database, also update control-plane's AR URL
    if [ "$prefix" = "ar" ]; then
        write_secret "control_plane_ar_database_url" "postgres://${db_user}:${new_pw}@${db_host}:5432/${db_name}"
        echo "  Also updated: control_plane_ar_database_url"
    fi

    echo ""
    echo "IMPORTANT: You must also update the Postgres role password:"
    echo "  docker exec ${db_host} psql -U ${db_user} -d ${db_name} \\"
    echo "    -c \"ALTER ROLE ${db_user} WITH PASSWORD '${new_pw}'\""
    echo ""
    echo "Then redeploy affected services."
}

rotate_nats() {
    local new_token
    new_token="$(gen_password)"
    echo "Rotating NATS auth token"
    write_secret "nats_auth_token" "$new_token"
    write_secret "nats_url" "nats://platform:${new_token}@7d-nats:4222"
    echo ""
    echo "Redeploy NATS and ALL backend services:"
    echo "  docker compose -f docker-compose.data.yml -f docker-compose.production.yml up -d nats"
    echo "  docker compose -f docker-compose.services.yml -f docker-compose.production.yml up -d"
}

rotate_jwt() {
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    openssl genpkey -algorithm ed25519 -out "${tmpdir}/private.pem" 2>/dev/null
    openssl pkey -in "${tmpdir}/private.pem" -pubout -out "${tmpdir}/public.pem" 2>/dev/null

    echo "Rotating JWT key pair"
    write_secret "jwt_private_key_pem" "$(cat "${tmpdir}/private.pem")"
    write_secret "jwt_public_key_pem" "$(cat "${tmpdir}/public.pem")"
    echo ""
    echo "WARNING: All existing JWTs will be invalidated."
    echo "Redeploy auth + all services that verify JWTs (gl, party, maintenance)."
}

show_list() {
    echo "Rotatable secrets:"
    echo ""
    echo "  nats              NATS auth token + all service NATS URLs"
    echo "  jwt               JWT signing key pair (invalidates all tokens)"
    echo "  db <prefix>       Single database password + URL"
    echo "  db all            All database passwords + URLs"
    echo ""
    echo "Database prefixes:"
    for prefix in $(echo "${!DB_MAP[@]}" | tr ' ' '\n' | sort); do
        echo "    ${prefix}"
    done
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
if [ $# -eq 0 ]; then
    echo "Usage: secrets_rotate.sh <nats|jwt|db <prefix>|--list>" >&2
    exit 1
fi

case "$1" in
    --list)
        show_list
        ;;
    nats)
        rotate_nats
        ;;
    jwt)
        rotate_jwt
        ;;
    db)
        if [ $# -lt 2 ]; then
            echo "Usage: secrets_rotate.sh db <prefix|all>" >&2
            exit 1
        fi
        if [ "$2" = "all" ]; then
            for prefix in $(echo "${!DB_MAP[@]}" | tr ' ' '\n' | sort); do
                rotate_db "$prefix"
                echo ""
            done
        else
            rotate_db "$2"
        fi
        ;;
    *)
        echo "Unknown command: $1" >&2
        echo "Use --list to see available secrets." >&2
        exit 1
        ;;
esac
