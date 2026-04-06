#!/usr/bin/env bash
# verify-create-app.sh — Phase 1 exit gate for the create-7d-app scaffold.
#
# Verifies:
#   1. CLI runs and produces all expected files with substitutions applied
#   2. pnpm install resolves workspace packages
#   3. TypeScript typecheck passes with zero errors
#   4. @7d/tokens CSS files are present and accessible
#   5. Brand substitution is applied correctly in layout + tailwind config
#   6. data-brand attribute present in <html> for runtime theme activation
#   7. Auth flow works against real identity-auth (register → login → token)
#
# Prerequisites: node >= 22, pnpm, jq, curl, python3 in PATH.
# Services required: identity-auth (port 8080), control-plane (port 8091).
#
# Usage: ./tooling/scripts/verify-create-app.sh [--no-cleanup]
#
# Exit: 0 = all checks passed, 1 = one or more checks failed.

set -euo pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

AUTH_LB_URL="${AUTH_LB_URL:-http://localhost:8080}"
CONTROL_PLANE_URL="${CONTROL_PLANE_URL:-http://localhost:8091}"
NO_CLEANUP="${1:-}"

# Sandbox lives inside apps/ so pnpm workspace picks it up.
SANDBOX_NAME="verify-sandbox-$$"
SANDBOX_DIR="$PROJECT_ROOT/apps/$SANDBOX_NAME"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

PASS=0
FAIL=0
FAILURES=()

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); FAILURES+=("$1"); }

section() { echo ""; echo "── $1 ──────────────────────────────"; }

require_cmd() {
  if ! command -v "$1" &>/dev/null; then
    echo "ERROR: required command '$1' not found in PATH" >&2
    exit 1
  fi
}

cleanup() {
  if [[ "$NO_CLEANUP" != "--no-cleanup" ]]; then
    if [[ -d "$SANDBOX_DIR" ]]; then
      trash "$SANDBOX_DIR" 2>/dev/null || true
      # Re-run pnpm install to remove the sandbox from pnpm-lock.yaml
      cd "$PROJECT_ROOT" && pnpm install --silent 2>/dev/null || true
    fi
  else
    echo ""
    echo "  (sandbox preserved at: $SANDBOX_DIR)"
  fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------

section "Prerequisites"

for cmd in node pnpm jq curl python3; do
  if command -v "$cmd" &>/dev/null; then
    pass "$cmd available"
  else
    fail "$cmd not found"
  fi
done

NODE_VER=$(node --version 2>/dev/null | sed 's/v//' | cut -d. -f1)
if [[ "$NODE_VER" -ge 22 ]]; then
  pass "node >= 22 (v$NODE_VER)"
else
  fail "node < 22 (v$NODE_VER) — native TypeScript execution requires >= 22"
fi

# Check services
if curl -sf "$AUTH_LB_URL/api/auth/login" -X POST -H "Content-Type: application/json" \
     -d '{}' 2>/dev/null | grep -q "missing field\|tenant_id"; then
  pass "identity-auth reachable at $AUTH_LB_URL"
else
  AUTH_HEALTH=$(curl -so /dev/null -w "%{http_code}" "$AUTH_LB_URL/api/auth/login" \
    -X POST -H "Content-Type: application/json" -d '{}' 2>/dev/null || echo "000")
  # 422/400/503 are all "service is up" — 000 means no connection
  if [[ "$AUTH_HEALTH" != "000" ]]; then
    pass "identity-auth reachable at $AUTH_LB_URL (HTTP $AUTH_HEALTH)"
  else
    fail "identity-auth not reachable at $AUTH_LB_URL"
  fi
fi

if curl -sf "$CONTROL_PLANE_URL/healthz" &>/dev/null || \
   curl -so /dev/null -w "%{http_code}" "$CONTROL_PLANE_URL/healthz" 2>/dev/null | grep -qv "^000"; then
  pass "control-plane reachable at $CONTROL_PLANE_URL"
else
  fail "control-plane not reachable at $CONTROL_PLANE_URL"
fi

# ---------------------------------------------------------------------------
# 1. CLI — scaffold with default brand (trashtech)
# ---------------------------------------------------------------------------

section "1. CLI scaffold (brand=trashtech)"

CLI="$PROJECT_ROOT/packages/create-app/create-7d-app.ts"
[[ -f "$CLI" ]] || { fail "CLI not found at $CLI"; exit 1; }

CLI_OUT=$(node --experimental-strip-types "$CLI" \
  "$SANDBOX_NAME" \
  --brand trashtech \
  --api-url "http://localhost:3001" \
  --dir "$SANDBOX_DIR" 2>&1)

if [[ -d "$SANDBOX_DIR" ]]; then
  pass "CLI created output directory"
else
  fail "CLI did not create output directory"
  echo "CLI output: $CLI_OUT"
  exit 1
fi

# Check expected files
EXPECTED_FILES=(
  "package.json"
  "next.config.ts"
  "tailwind.config.ts"
  "tsconfig.json"
  "eslint.config.js"
  "postcss.config.mjs"
  ".env.local"
  "app/layout.tsx"
  "app/page.tsx"
  "app/globals.css"
  "app/providers.tsx"
  "app/(auth)/login/page.tsx"
)

for f in "${EXPECTED_FILES[@]}"; do
  if [[ -f "$SANDBOX_DIR/$f" ]]; then
    pass "file exists: $f"
  else
    fail "missing file: $f"
  fi
done

# ---------------------------------------------------------------------------
# 2. Substitutions
# ---------------------------------------------------------------------------

section "2. Token substitutions"

check_sub() {
  local file="$1" expected="$2" label="$3"
  if grep -qF "$expected" "$SANDBOX_DIR/$file" 2>/dev/null; then
    pass "$label in $file"
  else
    fail "$label missing in $file (expected: $expected)"
  fi
}

check_not_present() {
  local file="$1" token="$2" label="$3"
  if grep -qF "$token" "$SANDBOX_DIR/$file" 2>/dev/null; then
    fail "unreplaced token $token still present in $file"
  else
    pass "$label — no unreplaced tokens in $file"
  fi
}

check_sub "package.json"    "\"name\": \"$SANDBOX_NAME\""       "app name in package.json"
check_sub "app/layout.tsx"  "import \"@7d/tokens/themes/trashtech\"" "brand theme import in layout.tsx"
check_sub "app/layout.tsx"  "data-brand=\"trashtech\""          "data-brand attribute in layout.tsx"
check_sub "app/layout.tsx"  "title: \"$(echo "$SANDBOX_NAME" | sed 's/-/ /g' | python3 -c "import sys; print(' '.join(w.capitalize() for w in sys.stdin.read().strip().split()))")\"" \
  "app title in layout.tsx"
check_sub ".env.local"      "NEXT_PUBLIC_PLATFORM_API_URL=http://localhost:3001" "API URL in .env.local"

# Verify no __PLACEHOLDER__ tokens remain
for f in "app/layout.tsx" "app/page.tsx" "package.json" "tailwind.config.ts"; do
  check_not_present "$f" "__APP_NAME__"     "no __APP_NAME__ in $f"
  check_not_present "$f" "__BRAND_THEME__"  "no __BRAND_THEME__ in $f"
  check_not_present "$f" "__API_URL__"      "no __API_URL__ in $f"
done

# ---------------------------------------------------------------------------
# 3. Brand override — scaffold with alternate brand
# ---------------------------------------------------------------------------

section "3. Brand override (brand=huberpower)"

ALT_SANDBOX_NAME="verify-sandbox-alt-$$"
ALT_SANDBOX_DIR="$PROJECT_ROOT/apps/$ALT_SANDBOX_NAME"

node --experimental-strip-types "$CLI" \
  "$ALT_SANDBOX_NAME" \
  --brand huberpower \
  --api-url "http://localhost:3001" \
  --dir "$ALT_SANDBOX_DIR" 2>&1 >/dev/null

if grep -qF 'import "@7d/tokens/themes/huberpower"' "$ALT_SANDBOX_DIR/app/layout.tsx" 2>/dev/null; then
  pass "huberpower brand import in layout.tsx"
else
  fail "huberpower brand import missing in layout.tsx"
fi

if grep -qF 'data-brand="huberpower"' "$ALT_SANDBOX_DIR/app/layout.tsx" 2>/dev/null; then
  pass "data-brand=huberpower in layout.tsx"
else
  fail "data-brand=huberpower missing in layout.tsx"
fi

# Cleanup alt sandbox immediately
trash "$ALT_SANDBOX_DIR" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 4. Token CSS files accessible
# ---------------------------------------------------------------------------

section "4. Design token CSS files"

TOKENS_PKG="$PROJECT_ROOT/packages/tokens/src"

for f in "tokens.css" "themes/trashtech.css" "themes/huberpower.css" "themes/ranchorbit.css"; do
  if [[ -f "$TOKENS_PKG/$f" ]]; then
    pass "@7d/tokens/$f present"
  else
    fail "@7d/tokens/$f missing"
  fi
done

# Verify tokens.css defines expected CSS custom properties
if grep -q "\-\-color-primary" "$TOKENS_PKG/tokens.css" 2>/dev/null; then
  pass "tokens.css defines --color-primary"
else
  fail "tokens.css missing --color-primary custom property"
fi

# Verify brand theme CSS uses data-brand selector
if grep -q '\[data-brand="trashtech"\]' "$TOKENS_PKG/themes/trashtech.css" 2>/dev/null; then
  pass "trashtech.css uses [data-brand] selector"
else
  fail "trashtech.css missing [data-brand=\"trashtech\"] selector"
fi

# ---------------------------------------------------------------------------
# 5. pnpm install + typecheck
# ---------------------------------------------------------------------------

section "5. Install and typecheck"

echo "  Running pnpm install..."
INSTALL_OUT=$(cd "$PROJECT_ROOT" && pnpm install 2>&1)
if [[ $? -eq 0 ]]; then
  pass "pnpm install succeeded"
else
  fail "pnpm install failed"
  echo "$INSTALL_OUT" | tail -20
fi

echo "  Running typecheck..."
TYPECHECK_OUT=$(cd "$SANDBOX_DIR" && pnpm typecheck 2>&1)
TYPECHECK_EXIT=$?
if [[ $TYPECHECK_EXIT -eq 0 ]]; then
  pass "TypeScript typecheck passed"
else
  fail "TypeScript typecheck failed (exit $TYPECHECK_EXIT)"
  echo "$TYPECHECK_OUT"
fi

# Verify platform-client and tokens are symlinked in node_modules
for pkg in "@7d/platform-client" "@7d/tokens" "@7d/ui"; do
  PKG_PATH="$SANDBOX_DIR/node_modules/$pkg"
  if [[ -d "$PKG_PATH" ]] || [[ -L "$PKG_PATH" ]]; then
    pass "$pkg in node_modules"
  else
    fail "$pkg not found in node_modules"
  fi
done

# ---------------------------------------------------------------------------
# 6. Auth flow against real identity-auth
# ---------------------------------------------------------------------------

section "6. Auth flow (real identity-auth)"

# Step 1: Provision a test tenant via control-plane
IDEM_KEY="verify-create-app-$$-$(date +%s)"
PROVISION_RESP=$(curl -sf -X POST "$CONTROL_PLANE_URL/api/control/tenants" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"$IDEM_KEY\",\"environment\":\"development\",\"product_code\":\"starter\",\"plan_code\":\"monthly\"}" 2>&1)

PROVISION_OK=$?
if [[ $PROVISION_OK -eq 0 ]]; then
  pass "tenant provisioning request accepted"
else
  fail "tenant provisioning failed: $PROVISION_RESP"
fi

TEST_TENANT_ID=$(echo "$PROVISION_RESP" | jq -r '.tenant_id // empty' 2>/dev/null)
if [[ -z "$TEST_TENANT_ID" ]]; then
  fail "could not extract tenant_id from provisioning response"
fi

# Step 2: Wait for tenant to reach 'active' status (up to 15s)
TENANT_ACTIVE=false
for i in $(seq 1 15); do
  STATUS=$(curl -sf "$CONTROL_PLANE_URL/api/tenants/$TEST_TENANT_ID/status" 2>/dev/null \
    | jq -r '.status // empty' 2>/dev/null)
  if [[ "$STATUS" == "active" ]]; then
    TENANT_ACTIVE=true
    break
  fi
  sleep 1
done

if [[ "$TENANT_ACTIVE" == "true" ]]; then
  pass "tenant provisioning completed (status=active)"
else
  fail "tenant did not reach 'active' within 15s (last status: ${STATUS:-unknown})"
fi

# Step 3: Register a test user
TEST_USER_ID=$(python3 -c 'import uuid; print(uuid.uuid4())')
TEST_EMAIL="verify-$$-${TEST_USER_ID}@7d-verify.test"
TEST_PASSWORD="VerifyTest2026Abc"

REG_RESP=$(curl -sf -X POST "$AUTH_LB_URL/api/auth/register" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TEST_TENANT_ID\",\"user_id\":\"$TEST_USER_ID\",\"email\":\"$TEST_EMAIL\",\"password\":\"$TEST_PASSWORD\"}" 2>&1)

if echo "$REG_RESP" | jq -e '.ok == true' &>/dev/null; then
  pass "user registration succeeded"
else
  fail "user registration failed: $REG_RESP"
fi

# Step 4: Login
LOGIN_RESP=$(curl -sf -X POST "$AUTH_LB_URL/api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TEST_TENANT_ID\",\"email\":\"$TEST_EMAIL\",\"password\":\"$TEST_PASSWORD\"}" 2>&1)

ACCESS_TOKEN=$(echo "$LOGIN_RESP" | jq -r '.access_token // empty' 2>/dev/null)
REFRESH_TOKEN=$(echo "$LOGIN_RESP" | jq -r '.refresh_token // empty' 2>/dev/null)
TOKEN_TYPE=$(echo "$LOGIN_RESP" | jq -r '.token_type // empty' 2>/dev/null)

if [[ -n "$ACCESS_TOKEN" ]]; then
  pass "login returned access_token"
else
  fail "login failed or no access_token: $LOGIN_RESP"
fi

if [[ -n "$REFRESH_TOKEN" ]]; then
  pass "login returned refresh_token"
else
  fail "login returned no refresh_token"
fi

if [[ "$TOKEN_TYPE" == "Bearer" ]]; then
  pass "token_type=Bearer"
else
  fail "unexpected token_type: $TOKEN_TYPE"
fi

# Step 5: Decode JWT payload and verify claims
if [[ -n "$ACCESS_TOKEN" ]]; then
  # JWT payload is the second dot-separated segment, base64url-encoded
  JWT_PAYLOAD=$(echo "$ACCESS_TOKEN" | cut -d. -f2)
  # Pad base64url to valid base64
  PADDED="$JWT_PAYLOAD$(python3 -c "n=len('$JWT_PAYLOAD')%4; print('='*(4-n) if n else '')")"
  DECODED=$(echo "$PADDED" | python3 -c "
import sys, base64, json
try:
    raw = sys.stdin.read().strip()
    padded = raw + '=' * (4 - len(raw) % 4)
    data = base64.urlsafe_b64decode(padded)
    print(json.dumps(json.loads(data)))
except Exception as e:
    print('{}')" 2>/dev/null)

  JWT_SUB=$(echo "$DECODED" | jq -r '.sub // empty' 2>/dev/null)
  JWT_ISS=$(echo "$DECODED" | jq -r '.iss // empty' 2>/dev/null)

  if [[ -n "$JWT_SUB" ]]; then
    pass "JWT has sub claim: $JWT_SUB"
  else
    fail "JWT missing sub claim"
  fi

  if [[ -n "$JWT_ISS" ]]; then
    pass "JWT has iss claim: $JWT_ISS"
  else
    fail "JWT missing iss claim"
  fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "══════════════════════════════════════════"
echo "  Results: $PASS passed, $FAIL failed"
echo "══════════════════════════════════════════"

if [[ $FAIL -gt 0 ]]; then
  echo ""
  echo "Failed checks:"
  for f in "${FAILURES[@]}"; do
    echo "  ✗ $f"
  done
  echo ""
  echo "Phase 1 exit gate: FAIL"
  exit 1
else
  echo ""
  echo "Phase 1 exit gate: PASS"
  exit 0
fi
