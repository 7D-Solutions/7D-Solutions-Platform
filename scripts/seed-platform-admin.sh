#!/usr/bin/env bash
# ============================================================
# seed-platform-admin.sh
# Creates a platform admin account in identity-auth for TCP UI access.
#
# Usage:
#   ./scripts/seed-platform-admin.sh                  # interactive prompts
#   ./scripts/seed-platform-admin.sh --email james@7dsolutions.app --password 'PlatformAdmin1'
#
# Environment overrides:
#   AUTH_URL           identity-auth base URL  (default: http://localhost:8080)
#   AUTH_DB_CONTAINER  postgres container name  (default: 7d-auth-postgres)
#   AUTH_DB_NAME       database name            (default: auth_db)
#   AUTH_DB_USER       database user            (default: auth_user)
# ============================================================
set -euo pipefail

# ── Well-known constants ─────────────────────────────────────
PLATFORM_TENANT_ID="00000000-0000-0000-0000-000000000000"

# ── Defaults ─────────────────────────────────────────────────
AUTH_URL="${AUTH_URL:-http://localhost:8080}"
AUTH_DB_CONTAINER="${AUTH_DB_CONTAINER:-7d-auth-postgres}"
AUTH_DB_NAME="${AUTH_DB_NAME:-auth_db}"
AUTH_DB_USER="${AUTH_DB_USER:-auth_user}"

# ── Helper: run SQL against auth DB via docker exec ──────────
run_sql() {
  docker exec -i "$AUTH_DB_CONTAINER" \
    psql -U "$AUTH_DB_USER" -d "$AUTH_DB_NAME" -v ON_ERROR_STOP=1 "$@"
}

run_sql_quiet() {
  docker exec -i "$AUTH_DB_CONTAINER" \
    psql -U "$AUTH_DB_USER" -d "$AUTH_DB_NAME" -v ON_ERROR_STOP=1 -tAq "$@"
}

# ── Parse args ───────────────────────────────────────────────
EMAIL=""
PASSWORD=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --email)    EMAIL="$2"; shift 2 ;;
    --password) PASSWORD="$2"; shift 2 ;;
    --help|-h)
      echo "Usage: $0 [--email EMAIL] [--password PASSWORD]"
      echo ""
      echo "Creates a platform admin account for the TCP UI."
      echo "If email/password are not provided, you will be prompted."
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ── Prompt for missing values ────────────────────────────────
if [[ -z "$EMAIL" ]]; then
  read -rp "Admin email: " EMAIL
fi
if [[ -z "$PASSWORD" ]]; then
  read -rsp "Admin password: " PASSWORD
  echo
fi

if [[ -z "$EMAIL" || -z "$PASSWORD" ]]; then
  echo "ERROR: email and password are required." >&2
  exit 1
fi

# ── Preflight: verify container is running ───────────────────
if ! docker exec "$AUTH_DB_CONTAINER" pg_isready -U "$AUTH_DB_USER" -d "$AUTH_DB_NAME" >/dev/null 2>&1; then
  echo "ERROR: $AUTH_DB_CONTAINER is not running or not ready." >&2
  echo "Start it with: docker compose -f docker-compose.data.yml up -d" >&2
  exit 1
fi

# ── Generate a user_id ───────────────────────────────────────
USER_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')

echo "=== Seed Platform Admin ==="
echo "  Email:     $EMAIL"
echo "  User ID:   $USER_ID"
echo "  Tenant:    $PLATFORM_TENANT_ID (platform)"
echo "  Auth URL:  $AUTH_URL"
echo "  DB:        $AUTH_DB_CONTAINER/$AUTH_DB_NAME"
echo ""

# ── Step 1: Register user via identity-auth HTTP API ─────────
echo "→ Registering user in identity-auth..."

# Build JSON payload safely (avoids shell expansion issues with special chars)
JSON_PAYLOAD=$(python3 -c "
import json, sys
print(json.dumps({
    'tenant_id': '$PLATFORM_TENANT_ID',
    'user_id': '$USER_ID',
    'email': sys.argv[1],
    'password': sys.argv[2]
}))
" "$EMAIL" "$PASSWORD")

HTTP_CODE=$(curl -s -o /tmp/seed-admin-response.json -w "%{http_code}" \
  -X POST "${AUTH_URL}/api/auth/register" \
  -H "Content-Type: application/json" \
  -d "$JSON_PAYLOAD")

if [[ "$HTTP_CODE" == "200" ]]; then
  echo "  ✓ User registered successfully"
elif [[ "$HTTP_CODE" == "409" ]]; then
  echo "  ⚠ User already exists — looking up existing user_id..."
  USER_ID=$(run_sql_quiet -c \
    "SELECT user_id FROM credentials WHERE tenant_id = '${PLATFORM_TENANT_ID}' AND email = '${EMAIL}'")
  if [[ -z "$USER_ID" ]]; then
    echo "  ERROR: Could not find existing user in database." >&2
    exit 1
  fi
  echo "  ✓ Found existing user: $USER_ID"
else
  echo "  ERROR: Registration failed (HTTP $HTTP_CODE)" >&2
  cat /tmp/seed-admin-response.json >&2
  echo "" >&2
  exit 1
fi

# ── Step 2: Set up RBAC via SQL ──────────────────────────────
echo "→ Setting up RBAC (role, permissions, binding)..."

run_sql -q <<SQL
-- Create platform_admin role (idempotent)
INSERT INTO roles (tenant_id, name, description, is_system)
VALUES ('${PLATFORM_TENANT_ID}', 'platform_admin', 'Full platform administration', true)
ON CONFLICT (tenant_id, name) DO NOTHING;

-- Create platform permissions
INSERT INTO permissions (key, description) VALUES
  ('cp.read',    'Read tenant and platform data'),
  ('cp.mutate',  'Create and modify tenants'),
  ('cp.admin',   'Full platform administration'),
  ('cp.billing', 'View and manage billing'),
  ('cp.support', 'Access support sessions'),
  ('cp.audit',   'View audit logs')
ON CONFLICT (key) DO NOTHING;

-- Grant all cp.* permissions to platform_admin role
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
CROSS JOIN permissions p
WHERE r.tenant_id = '${PLATFORM_TENANT_ID}'
  AND r.name = 'platform_admin'
  AND p.key LIKE 'cp.%'
ON CONFLICT (role_id, permission_id) DO NOTHING;

-- Bind user to platform_admin role
INSERT INTO user_role_bindings (tenant_id, user_id, role_id)
SELECT '${PLATFORM_TENANT_ID}', '${USER_ID}', r.id
FROM roles r
WHERE r.tenant_id = '${PLATFORM_TENANT_ID}' AND r.name = 'platform_admin'
ON CONFLICT (tenant_id, user_id, role_id)
DO UPDATE SET revoked_at = NULL;
SQL

echo "  ✓ RBAC configured"

# ── Step 3: Verify ───────────────────────────────────────────
echo ""
echo "=== Verification ==="

ROLE_COUNT=$(run_sql_quiet -c \
  "SELECT count(*) FROM user_role_bindings urb
   JOIN roles r ON r.id = urb.role_id
   WHERE urb.tenant_id = '${PLATFORM_TENANT_ID}'
     AND urb.user_id = '${USER_ID}'
     AND r.name = 'platform_admin'
     AND urb.revoked_at IS NULL")

if [[ "$ROLE_COUNT" -ge 1 ]]; then
  echo "  ✓ User has platform_admin role"
else
  echo "  ✗ Role binding not found!" >&2
  exit 1
fi

PERM_COUNT=$(run_sql_quiet -c \
  "SELECT count(*) FROM role_permissions rp
   JOIN roles r ON r.id = rp.role_id
   WHERE r.tenant_id = '${PLATFORM_TENANT_ID}' AND r.name = 'platform_admin'")

echo "  ✓ platform_admin role has ${PERM_COUNT} permissions"

echo ""
echo "=== Done ==="
echo "Platform admin '${EMAIL}' is ready."
echo "Log in at http://localhost:3000 with these credentials."
