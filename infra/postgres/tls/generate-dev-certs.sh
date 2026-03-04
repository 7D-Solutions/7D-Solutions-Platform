#!/usr/bin/env bash
# generate-dev-certs.sh — Generate a self-signed CA and server certificate for
# local development Postgres TLS.  The server cert covers every 7d-*-postgres
# container hostname so a single cert works for all databases.
#
# Output goes to infra/postgres/tls/{ca.crt, server.crt, server.key}.
# These files are gitignored; run this script once after cloning.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Skip if certs already exist and are not expired
if [[ -f server.crt && -f server.key && -f ca.crt ]]; then
  if openssl x509 -checkend 86400 -noout -in server.crt 2>/dev/null; then
    echo "Dev TLS certs already exist and are valid. To regenerate, delete infra/postgres/tls/{ca.crt,server.crt,server.key} and re-run."
    exit 0
  fi
  echo "Existing cert is expiring within 24h — regenerating."
fi

echo "Generating dev CA + Postgres server certificate..."

# --- CA ---
openssl req -new -x509 -nodes \
  -days 3650 \
  -keyout ca.key \
  -out ca.crt \
  -subj "/CN=7D Dev CA/O=7D Solutions/OU=Development" \
  2>/dev/null

# --- Server key + CSR ---
openssl req -new -nodes \
  -keyout server.key \
  -out server.csr \
  -subj "/CN=7d-postgres/O=7D Solutions/OU=Development" \
  2>/dev/null

# Build SAN list covering all database container hostnames
HOSTNAMES=(
  7d-auth-postgres
  7d-ar-postgres
  7d-subscriptions-postgres
  7d-payments-postgres
  7d-notifications-postgres
  7d-gl-postgres
  7d-projections-postgres
  7d-audit-postgres
  7d-tenant-registry-postgres
  7d-inventory-postgres
  7d-ap-postgres
  7d-treasury-postgres
  7d-fixed-assets-postgres
  7d-consolidation-postgres
  7d-timekeeping-postgres
  7d-party-postgres
  7d-integrations-postgres
  7d-ttp-postgres
  7d-maintenance-postgres
  7d-pdf-editor-postgres
  7d-shipping-receiving-postgres
  7d-numbering-postgres
  7d-doc-mgmt-postgres
  7d-workflow-postgres
  7d-workforce-competence-postgres
  localhost
)

SAN_ENTRIES="DNS:${HOSTNAMES[0]}"
for h in "${HOSTNAMES[@]:1}"; do
  SAN_ENTRIES="${SAN_ENTRIES},DNS:${h}"
done
SAN_ENTRIES="${SAN_ENTRIES},IP:127.0.0.1"

# --- Sign server cert with CA ---
openssl x509 -req \
  -in server.csr \
  -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 3650 \
  -extfile <(printf "subjectAltName=%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth" "$SAN_ENTRIES") \
  -out server.crt \
  2>/dev/null

# Postgres requires server.key to be readable only by owner (mode 600).
# In Docker the file is mounted read-only; the entrypoint wrapper handles
# copying it to the right permissions.  Locally we set 600 for safety.
chmod 600 server.key

# Clean up intermediates
rm -f server.csr ca.key ca.srl

echo "Done. Generated:"
echo "  $SCRIPT_DIR/ca.crt       (CA certificate — mount in clients for verify-ca)"
echo "  $SCRIPT_DIR/server.crt   (server certificate)"
echo "  $SCRIPT_DIR/server.key   (server private key)"
