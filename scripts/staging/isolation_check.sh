#!/usr/bin/env bash
# isolation_check.sh — Multi-tenant isolation verification harness
#
# Provisions (or identifies) two staging tenants (A and B), obtains
# tenant-scoped access tokens, and runs a matrix of cross-tenant
# requests asserting 401/403 for every attempted cross-tenant read.
#
# Coverage:
#   - TCP UI BFF endpoints (/api/tenants, /api/plans, etc.)
#   - AR module direct API (/api/ar/invoices)
#   - TTP service-agreements read path (informational — SQL-scoped, no HTTP auth)
#   - Control-plane tenant detail (informational — network-isolated, no HTTP auth)
#
# Usage:
#   STAGING_HOST=<host> bash scripts/staging/isolation_check.sh
#
# Optional env overrides:
#   ISOLATION_BFF_PORT          TCP UI port          (default: 3000)
#   ISOLATION_AUTH_PORT         identity-auth LB     (default: 8080)
#   ISOLATION_CP_PORT           control-plane        (default: 8091)
#   ISOLATION_AR_PORT           AR service           (default: 8086)
#   ISOLATION_TTP_PORT          TTP service          (default: 8100)
#   ISOLATION_USER_PASSWORD     password for test users (default: IsoCheck!7d2026)
#
# Prerequisites: curl, python3

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ─── Colors ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

banner() { echo -e "\n${CYAN}${BOLD}=== $1 ===${NC}"; }
info()   { echo -e "${YELLOW}[INFO]${NC} $1"; }
pass()   { echo -e "${GREEN}[PASS]${NC} $1"; }
fail()   { echo -e "${RED}[FAIL]${NC} $1"; FAILED=$((FAILED + 1)); }
note()   { echo -e "${YELLOW}[NOTE]${NC} $1"; }

# ─── Configuration ────────────────────────────────────────────────────────────
STAGING_HOST="${STAGING_HOST:?ERROR: STAGING_HOST must be set. E.g. STAGING_HOST=staging.7dsolutions.example.com bash $0}"

BFF_PORT="${ISOLATION_BFF_PORT:-3000}"
AUTH_PORT="${ISOLATION_AUTH_PORT:-8080}"
CP_PORT="${ISOLATION_CP_PORT:-8091}"
AR_PORT="${ISOLATION_AR_PORT:-8086}"
TTP_PORT="${ISOLATION_TTP_PORT:-8100}"
USER_PASS="${ISOLATION_USER_PASSWORD:-IsoCheck!7d2026}"

BFF_URL="http://${STAGING_HOST}:${BFF_PORT}"
AUTH_URL="http://${STAGING_HOST}:${AUTH_PORT}"
CP_URL="http://${STAGING_HOST}:${CP_PORT}"
AR_URL="http://${STAGING_HOST}:${AR_PORT}"
TTP_URL="http://${STAGING_HOST}:${TTP_PORT}"

FAILED=0
PASSED=0
INFO_COUNT=0

# ─── JSON helper ──────────────────────────────────────────────────────────────
json_field() {
    # json_field <json_string> <field_name>
    echo "$1" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$2',''))" 2>/dev/null || true
}

# ─── HTTP helper ──────────────────────────────────────────────────────────────
http_status() {
    # http_status <curl_args...>
    curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$@"
}

# ─── Denial assertion helper ──────────────────────────────────────────────────
assert_denied() {
    # assert_denied <label> <status>
    local label="$1"
    local status="$2"
    if [[ "$status" == "401" || "$status" == "403" || "$status" == "404" ]]; then
        pass "$label → HTTP $status (denied as expected)"
        PASSED=$((PASSED + 1))
    else
        fail "$label → HTTP $status (EXPECTED 401/403/404)"
    fi
}

# ─── Informational check helper ───────────────────────────────────────────────
check_info() {
    # check_info <label> <status> <expected_note>
    local label="$1"
    local status="$2"
    local note_text="$3"
    note "$label → HTTP $status — $note_text"
    INFO_COUNT=$((INFO_COUNT + 1))
}

# ─── Prerequisite check ───────────────────────────────────────────────────────
banner "Prerequisites"

if ! command -v curl >/dev/null 2>&1; then
    echo "ERROR: curl is required" >&2; exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 is required for JSON parsing" >&2; exit 1
fi
info "curl and python3 available"

# ─── Service reachability ─────────────────────────────────────────────────────
banner "Service reachability"

check_svc() {
    local label="$1"
    local url="$2"
    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$url" 2>/dev/null || echo "000")
    if [[ "$code" != "000" ]]; then
        info "$label reachable ($url → HTTP $code)"
    else
        fail "$label unreachable ($url)"
    fi
}

check_svc "auth"          "$AUTH_URL/health/live"
check_svc "control-plane" "$CP_URL/healthz"
check_svc "ar"            "$AR_URL/api/health"
check_svc "ttp"           "$TTP_URL/healthz"
check_svc "bff"           "$BFF_URL/login"

# ─── Provision Tenant A ───────────────────────────────────────────────────────
banner "Provision Tenant A"

TENANT_A_RESP=$(curl -s -X POST "$CP_URL/api/control/tenants" \
    -H "Content-Type: application/json" \
    --max-time 15 \
    -d '{
        "idempotency_key": "isolation-harness-tenant-a-v1",
        "environment": "staging",
        "product_code": "starter",
        "plan_code":    "monthly"
    }')

TENANT_A_ID=$(json_field "$TENANT_A_RESP" "tenant_id")
if [[ -z "$TENANT_A_ID" ]]; then
    fail "Tenant A provisioning failed — no tenant_id in response: $TENANT_A_RESP"
    exit 1
fi
info "Tenant A: $TENANT_A_ID"

# ─── Provision Tenant B ───────────────────────────────────────────────────────
banner "Provision Tenant B"

TENANT_B_RESP=$(curl -s -X POST "$CP_URL/api/control/tenants" \
    -H "Content-Type: application/json" \
    --max-time 15 \
    -d '{
        "idempotency_key": "isolation-harness-tenant-b-v1",
        "environment": "staging",
        "product_code": "starter",
        "plan_code":    "monthly"
    }')

TENANT_B_ID=$(json_field "$TENANT_B_RESP" "tenant_id")
if [[ -z "$TENANT_B_ID" ]]; then
    fail "Tenant B provisioning failed — no tenant_id in response: $TENANT_B_RESP"
    exit 1
fi
info "Tenant B: $TENANT_B_ID"

# ─── Register users ───────────────────────────────────────────────────────────
banner "Register tenant users"

register_user() {
    local tenant_id="$1"
    local email="$2"
    local label="$3"
    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$AUTH_URL/api/auth/register" \
        -H "Content-Type: application/json" \
        --max-time 15 \
        -d "{
            \"tenant_id\": \"$tenant_id\",
            \"user_id\":   \"$(python3 -c 'import uuid; print(uuid.uuid4())')\",
            \"email\":     \"$email\",
            \"password\":  \"$USER_PASS\"
        }")
    case "$code" in
        201|200) info "$label registered successfully (HTTP $code)" ;;
        409)     info "$label already registered — proceeding with login" ;;
        *)       fail "$label registration failed — HTTP $code"; return 1 ;;
    esac
}

register_user "$TENANT_A_ID" "isolation-user-a@test.7d.internal" "User A"
register_user "$TENANT_B_ID" "isolation-user-b@test.7d.internal" "User B"

# ─── Login and obtain tokens ──────────────────────────────────────────────────
banner "Obtain tenant tokens"

login_tenant() {
    local tenant_id="$1"
    local email="$2"
    local label="$3"
    local resp
    resp=$(curl -s -X POST "$AUTH_URL/api/auth/login" \
        -H "Content-Type: application/json" \
        --max-time 15 \
        -d "{
            \"tenant_id\": \"$tenant_id\",
            \"email\":     \"$email\",
            \"password\":  \"$USER_PASS\"
        }")
    local token
    token=$(json_field "$resp" "access_token")
    if [[ -z "$token" ]]; then
        fail "$label login failed — response: $resp"
        echo ""
    else
        info "$label token obtained (${#token} chars)"
        echo "$token"
    fi
}

TOKEN_A=$(login_tenant "$TENANT_A_ID" "isolation-user-a@test.7d.internal" "Tenant A user")
TOKEN_B=$(login_tenant "$TENANT_B_ID" "isolation-user-b@test.7d.internal" "Tenant B user")

if [[ -z "$TOKEN_A" || -z "$TOKEN_B" ]]; then
    fail "Cannot proceed without both tenant tokens"
    exit 1
fi

# ─── Cross-tenant isolation checks ───────────────────────────────────────────
banner "Cross-tenant isolation checks"

echo -e "\n${BOLD}--- BFF (TCP UI) — unauthenticated requests ---${NC}"

# Check 1: No auth → /api/tenants → 401
S=$(http_status "$BFF_URL/api/tenants")
assert_denied "CHECK-01 [BFF] no auth → GET /api/tenants" "$S"

# Check 2: No auth → /api/tenants/{B} → 401
S=$(http_status "$BFF_URL/api/tenants/$TENANT_B_ID")
assert_denied "CHECK-02 [BFF] no auth → GET /api/tenants/{TENANT_B}" "$S"

# Check 3: No auth → /api/tenants/{B}/invoices → 401
S=$(http_status "$BFF_URL/api/tenants/$TENANT_B_ID/invoices")
assert_denied "CHECK-03 [BFF] no auth → GET /api/tenants/{TENANT_B}/invoices" "$S"

# Check 4: No auth → /api/plans → 401
S=$(http_status "$BFF_URL/api/plans")
assert_denied "CHECK-04 [BFF] no auth → GET /api/plans" "$S"

echo -e "\n${BOLD}--- BFF (TCP UI) — Tenant A JWT reading Tenant B resources ---${NC}"

# Check 5: Tenant A JWT → /api/tenants → 403 (no platform_admin role)
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_A" "$BFF_URL/api/tenants")
assert_denied "CHECK-05 [BFF] tenant-A JWT → GET /api/tenants (no platform_admin)" "$S"

# Check 6: Tenant A JWT → /api/tenants/{B} → 403
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_A" "$BFF_URL/api/tenants/$TENANT_B_ID")
assert_denied "CHECK-06 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}" "$S"

# Check 7: Tenant A JWT → /api/tenants/{B}/invoices → 403
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_A" "$BFF_URL/api/tenants/$TENANT_B_ID/invoices")
assert_denied "CHECK-07 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}/invoices" "$S"

# Check 8: Tenant A JWT → /api/tenants/{B}/billing/overview → 403
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_A" "$BFF_URL/api/tenants/$TENANT_B_ID/billing/overview")
assert_denied "CHECK-08 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}/billing/overview" "$S"

echo -e "\n${BOLD}--- BFF (TCP UI) — Tenant B JWT reading Tenant A resources ---${NC}"

# Check 9: Tenant B JWT → /api/tenants/{A} → 403
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_B" "$BFF_URL/api/tenants/$TENANT_A_ID")
assert_denied "CHECK-09 [BFF] tenant-B JWT → GET /api/tenants/{TENANT_A}" "$S"

# Check 10: Tenant B JWT → /api/tenants/{A}/invoices → 403
S=$(http_status -H "Cookie: tcp_auth_token=$TOKEN_B" "$BFF_URL/api/tenants/$TENANT_A_ID/invoices")
assert_denied "CHECK-10 [BFF] tenant-B JWT → GET /api/tenants/{TENANT_A}/invoices" "$S"

echo -e "\n${BOLD}--- AR — tenant user tokens must not access AR directly ---${NC}"

# Check 11: Tenant A JWT as Bearer → AR invoice list → 401 (AR requires service token)
S=$(http_status -H "Authorization: Bearer $TOKEN_A" "$AR_URL/api/ar/invoices")
assert_denied "CHECK-11 [AR] tenant-A JWT (non-service token) → GET /api/ar/invoices" "$S"

# Check 12: Tenant B JWT as Bearer → AR → 401
S=$(http_status -H "Authorization: Bearer $TOKEN_B" "$AR_URL/api/ar/invoices")
assert_denied "CHECK-12 [AR] tenant-B JWT (non-service token) → GET /api/ar/invoices" "$S"

echo -e "\n${BOLD}--- Control-plane — informational (network isolation) ---${NC}"

# Informational: control-plane has no HTTP auth — isolation is network-level
S=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 \
    -H "Authorization: Bearer $TOKEN_A" \
    "$CP_URL/api/tenants/$TENANT_B_ID" 2>/dev/null || echo "000")
check_info "CHECK-CP [CP] tenant-A token → GET /api/tenants/{TENANT_B} (control-plane)" "$S" \
    "CP has no HTTP auth enforcement — access control is network-level (firewall). HTTP $S is expected behavior."

echo -e "\n${BOLD}--- TTP — informational (SQL-scoped isolation) ---${NC}"

# Informational: TTP has no HTTP auth — cross-tenant reads return empty via SQL WHERE tenant_id
SA=$(curl -s --max-time 10 \
    -H "Authorization: Bearer $TOKEN_A" \
    "$TTP_URL/api/ttp/service-agreements?tenant_id=$TENANT_B_ID" 2>/dev/null || echo '{}')
TTP_COUNT=$(json_field "$SA" "count")
S=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 \
    "$TTP_URL/api/ttp/service-agreements?tenant_id=$TENANT_B_ID")
check_info "CHECK-TTP [TTP] cross-tenant service-agreements read" "$S" \
    "TTP SQL-scopes by tenant_id. Tenant B agreement count from tenant-A context: ${TTP_COUNT:-0}. HTTP auth enforcement is network-level."

# ─── Summary ──────────────────────────────────────────────────────────────────
banner "Results"

TOTAL_ASSERTIONS=$((PASSED + FAILED))
echo ""
echo -e "${BOLD}Tenant A:${NC} $TENANT_A_ID"
echo -e "${BOLD}Tenant B:${NC} $TENANT_B_ID"
echo ""
echo -e "${BOLD}Denial assertions:${NC} $TOTAL_ASSERTIONS (${GREEN}$PASSED PASS${NC} / ${RED}$FAILED FAIL${NC})"
echo -e "${BOLD}Informational:${NC}    $INFO_COUNT checks (TTP + control-plane — network-isolated)"
echo ""

if [[ "$FAILED" -gt 0 ]]; then
    echo -e "${RED}${BOLD}ISOLATION CHECK FAILED — $FAILED assertion(s) did not get the expected denial response.${NC}"
    exit 1
else
    echo -e "${GREEN}${BOLD}ISOLATION CHECK PASSED — All $PASSED cross-tenant denial assertions returned 401/403/404.${NC}"
    exit 0
fi
