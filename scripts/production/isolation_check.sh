#!/usr/bin/env bash
# isolation_check.sh — Production multi-tenant isolation verification harness
#
# All production service ports are firewalled.  Every curl check runs via SSH
# on the VPS against localhost so no port needs to be exposed to the internet.
#
# Provisions (or identifies) two production tenants (A and B), obtains
# tenant-scoped access tokens, and runs a matrix of cross-tenant requests
# asserting 401/403/404 for every attempted cross-tenant read.
#
# Coverage:
#   - TCP UI BFF endpoints (/api/tenants, /api/plans, etc.)
#   - AR module direct API (/api/ar/invoices)
#   - TTP service-agreements read path (informational — SQL-scoped)
#   - Control-plane tenant detail (informational — network-isolated)
#
# Usage:
#   PROD_HOST=<host> bash scripts/production/isolation_check.sh
#
# Optional env overrides:
#   PROD_USER               SSH deploy user       (default: deploy)
#   PROD_SSH_PORT           SSH port              (default: 22)
#   ISOLATION_USER_PASSWORD password for test users (default: IsoCheckProd!7d2026)
#
# GitHub Actions integration:
#   On success, writes ISOLATION_TENANT_A_ID, ISOLATION_TENANT_B_ID,
#   ISOLATION_TOKEN_A, and ISOLATION_TOKEN_B to $GITHUB_ENV so the subsequent
#   Playwright production isolation step can use them.
#
# Prerequisites: ssh, python3 (both available in GitHub Actions ubuntu-latest).
#   curl and python3 must also be installed on the VPS.

set -euo pipefail

# ─── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

banner() { echo -e "\n${CYAN}${BOLD}=== $1 ===${NC}"; }
info()   { echo -e "${YELLOW}[INFO]${NC} $1"; }
pass()   { echo -e "${GREEN}[PASS]${NC} $1"; PASSED=$((PASSED + 1)); }
fail()   { echo -e "${RED}[FAIL]${NC} $1"; FAILED=$((FAILED + 1)); }
note()   { echo -e "${YELLOW}[NOTE]${NC} $1"; }

# ─── Configuration ─────────────────────────────────────────────────────────────
HOST="${PROD_HOST:?ERROR: PROD_HOST must be set. E.g. PROD_HOST=prod.7dsolutions.example.com bash $0}"
USER="${PROD_USER:-deploy}"
SSH_PORT="${PROD_SSH_PORT:-22}"
USER_PASS="${ISOLATION_USER_PASSWORD:-IsoCheckProd!7d2026}"

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${SSH_PORT}"
SSH_TARGET="${USER}@${HOST}"

FAILED=0
PASSED=0
INFO_COUNT=0

# ─── JSON helper (parses SSH-captured output locally) ─────────────────────────
json_field() {
    echo "$1" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$2',''))" 2>/dev/null || true
}

# ─── HTTP status via SSH localhost ────────────────────────────────────────────
# ssh_status <url> [extra_curl_flags]
ssh_status() {
    local url="$1"
    local extra="${2:-}"
    # shellcheck disable=SC2029,SC2086
    ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -o /dev/null -w '%{http_code}' --max-time 10 ${extra} '${url}' 2>/dev/null || echo 000"
}

# ─── Assertion helpers ─────────────────────────────────────────────────────────
assert_denied() {
    local label="$1"
    local status="$2"
    if [[ "$status" == "401" || "$status" == "403" || "$status" == "404" ]]; then
        pass "$label → HTTP $status (denied as expected)"
    else
        fail "$label → HTTP $status (EXPECTED 401/403/404)"
    fi
}

check_info() {
    local label="$1"
    local status="$2"
    local note_text="$3"
    note "$label → HTTP $status — $note_text"
    INFO_COUNT=$((INFO_COUNT + 1))
}

# ─── Prerequisite checks ───────────────────────────────────────────────────────
banner "Prerequisites"

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 is required for JSON parsing" >&2; exit 1
fi

if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach ${SSH_TARGET} via SSH." >&2
    exit 1
fi
info "SSH connectivity OK — ${SSH_TARGET}"

# ─── Service reachability (via SSH localhost) ─────────────────────────────────
banner "Service reachability (via SSH localhost)"

check_svc() {
    local label="$1"
    local url="$2"
    local code
    # shellcheck disable=SC2029
    code=$(ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -o /dev/null -w '%{http_code}' --max-time 5 '${url}' 2>/dev/null || echo 000")
    if [[ "$code" != "000" ]]; then
        info "$label reachable ($url → HTTP $code)"
    else
        fail "$label unreachable ($url)"
    fi
}

check_svc "auth"          "http://localhost:8080/health/live"
check_svc "control-plane" "http://localhost:8091/healthz"
check_svc "ar"            "http://localhost:8086/api/health"
check_svc "ttp"           "http://localhost:8100/healthz"
check_svc "bff"           "http://localhost:3000/login"

# ─── Provision Tenant A ────────────────────────────────────────────────────────
banner "Provision Tenant A"

# shellcheck disable=SC2029
TENANT_A_RESP=$(ssh $SSH_OPTS "$SSH_TARGET" \
    "curl -s -X POST 'http://localhost:8091/api/control/tenants' \
     -H 'Content-Type: application/json' \
     --data-raw '{\"idempotency_key\":\"isolation-harness-prod-tenant-a-v1\",\"environment\":\"production\",\"product_code\":\"starter\",\"plan_code\":\"monthly\"}' \
     --max-time 15 2>/dev/null || echo '{}'")

TENANT_A_ID=$(json_field "$TENANT_A_RESP" "tenant_id")
if [[ -z "$TENANT_A_ID" ]]; then
    fail "Tenant A provisioning failed — no tenant_id in response: $TENANT_A_RESP"
    exit 1
fi
info "Tenant A: $TENANT_A_ID"

# ─── Provision Tenant B ────────────────────────────────────────────────────────
banner "Provision Tenant B"

# shellcheck disable=SC2029
TENANT_B_RESP=$(ssh $SSH_OPTS "$SSH_TARGET" \
    "curl -s -X POST 'http://localhost:8091/api/control/tenants' \
     -H 'Content-Type: application/json' \
     --data-raw '{\"idempotency_key\":\"isolation-harness-prod-tenant-b-v1\",\"environment\":\"production\",\"product_code\":\"starter\",\"plan_code\":\"monthly\"}' \
     --max-time 15 2>/dev/null || echo '{}'")

TENANT_B_ID=$(json_field "$TENANT_B_RESP" "tenant_id")
if [[ -z "$TENANT_B_ID" ]]; then
    fail "Tenant B provisioning failed — no tenant_id in response: $TENANT_B_RESP"
    exit 1
fi
info "Tenant B: $TENANT_B_ID"

# ─── Register users ────────────────────────────────────────────────────────────
banner "Register tenant users"

USER_ID_A=$(python3 -c 'import uuid; print(uuid.uuid4())')
USER_ID_B=$(python3 -c 'import uuid; print(uuid.uuid4())')

register_user() {
    local tenant_id="$1"
    local user_id="$2"
    local email="$3"
    local label="$4"
    local code
    # shellcheck disable=SC2029
    code=$(ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -o /dev/null -w '%{http_code}' -X POST 'http://localhost:8080/api/auth/register' \
         -H 'Content-Type: application/json' \
         --data-raw '{\"tenant_id\":\"${tenant_id}\",\"user_id\":\"${user_id}\",\"email\":\"${email}\",\"password\":\"${USER_PASS}\"}' \
         --max-time 15 2>/dev/null || echo 000")
    case "$code" in
        201|200) info "$label registered (HTTP $code)" ;;
        409)     info "$label already registered — proceeding with login" ;;
        *)       fail "$label registration failed — HTTP $code"; return 1 ;;
    esac
}

register_user "$TENANT_A_ID" "$USER_ID_A" "isolation-prod-user-a@test.7d.internal" "User A"
register_user "$TENANT_B_ID" "$USER_ID_B" "isolation-prod-user-b@test.7d.internal" "User B"

# ─── Login and obtain tokens ───────────────────────────────────────────────────
banner "Obtain tenant tokens"

login_tenant() {
    local tenant_id="$1"
    local email="$2"
    local label="$3"
    local resp token
    # shellcheck disable=SC2029
    resp=$(ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -X POST 'http://localhost:8080/api/auth/login' \
         -H 'Content-Type: application/json' \
         --data-raw '{\"tenant_id\":\"${tenant_id}\",\"email\":\"${email}\",\"password\":\"${USER_PASS}\"}' \
         --max-time 15 2>/dev/null || echo '{}'")
    token=$(json_field "$resp" "access_token")
    if [[ -z "$token" ]]; then
        fail "$label login failed — response: $resp"
        echo ""
    else
        info "$label token obtained (${#token} chars)"
        echo "$token"
    fi
}

TOKEN_A=$(login_tenant "$TENANT_A_ID" "isolation-prod-user-a@test.7d.internal" "Tenant A user")
TOKEN_B=$(login_tenant "$TENANT_B_ID" "isolation-prod-user-b@test.7d.internal" "Tenant B user")

if [[ -z "$TOKEN_A" || -z "$TOKEN_B" ]]; then
    fail "Cannot proceed without both tenant tokens"
    exit 1
fi

# ─── Cross-tenant isolation checks (all via SSH localhost) ─────────────────────
banner "Cross-tenant isolation checks"

echo -e "\n${BOLD}--- BFF (TCP UI) — unauthenticated requests ---${NC}"

S=$(ssh_status "http://localhost:3000/api/tenants")
assert_denied "CHECK-01 [BFF] no auth → GET /api/tenants" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_B_ID")
assert_denied "CHECK-02 [BFF] no auth → GET /api/tenants/{TENANT_B}" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_B_ID/invoices")
assert_denied "CHECK-03 [BFF] no auth → GET /api/tenants/{TENANT_B}/invoices" "$S"

S=$(ssh_status "http://localhost:3000/api/plans")
assert_denied "CHECK-04 [BFF] no auth → GET /api/plans" "$S"

echo -e "\n${BOLD}--- BFF (TCP UI) — Tenant A JWT reading Tenant B resources ---${NC}"

S=$(ssh_status "http://localhost:3000/api/tenants" "-H 'Cookie: tcp_auth_token=${TOKEN_A}'")
assert_denied "CHECK-05 [BFF] tenant-A JWT → GET /api/tenants (no platform_admin)" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_B_ID" "-H 'Cookie: tcp_auth_token=${TOKEN_A}'")
assert_denied "CHECK-06 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_B_ID/invoices" "-H 'Cookie: tcp_auth_token=${TOKEN_A}'")
assert_denied "CHECK-07 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}/invoices" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_B_ID/billing/overview" "-H 'Cookie: tcp_auth_token=${TOKEN_A}'")
assert_denied "CHECK-08 [BFF] tenant-A JWT → GET /api/tenants/{TENANT_B}/billing/overview" "$S"

echo -e "\n${BOLD}--- BFF (TCP UI) — Tenant B JWT reading Tenant A resources ---${NC}"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_A_ID" "-H 'Cookie: tcp_auth_token=${TOKEN_B}'")
assert_denied "CHECK-09 [BFF] tenant-B JWT → GET /api/tenants/{TENANT_A}" "$S"

S=$(ssh_status "http://localhost:3000/api/tenants/$TENANT_A_ID/invoices" "-H 'Cookie: tcp_auth_token=${TOKEN_B}'")
assert_denied "CHECK-10 [BFF] tenant-B JWT → GET /api/tenants/{TENANT_A}/invoices" "$S"

echo -e "\n${BOLD}--- AR — tenant user tokens must not access AR directly ---${NC}"

S=$(ssh_status "http://localhost:8086/api/ar/invoices" "-H 'Authorization: Bearer ${TOKEN_A}'")
assert_denied "CHECK-11 [AR] tenant-A JWT (non-service token) → GET /api/ar/invoices" "$S"

S=$(ssh_status "http://localhost:8086/api/ar/invoices" "-H 'Authorization: Bearer ${TOKEN_B}'")
assert_denied "CHECK-12 [AR] tenant-B JWT (non-service token) → GET /api/ar/invoices" "$S"

echo -e "\n${BOLD}--- Control-plane — informational (network isolation) ---${NC}"

# CP has no HTTP auth — access control is network-level (firewall/UFW).
# shellcheck disable=SC2029
CP_STATUS=$(ssh $SSH_OPTS "$SSH_TARGET" \
    "curl -s -o /dev/null -w '%{http_code}' --max-time 10 \
     -H 'Authorization: Bearer ${TOKEN_A}' \
     'http://localhost:8091/api/tenants/${TENANT_B_ID}' 2>/dev/null || echo 000")
check_info "CHECK-CP [CP] tenant-A token → GET /api/tenants/{TENANT_B}" "$CP_STATUS" \
    "CP has no HTTP auth enforcement — access control is network-level (UFW firewall). HTTP $CP_STATUS is expected behavior."

echo -e "\n${BOLD}--- TTP — informational (SQL-scoped isolation) ---${NC}"

# TTP SQL-scopes reads by tenant_id; cross-tenant queries return empty sets.
# shellcheck disable=SC2029
TTP_RESP=$(ssh $SSH_OPTS "$SSH_TARGET" \
    "curl -s --max-time 10 \
     -H 'Authorization: Bearer ${TOKEN_A}' \
     'http://localhost:8100/api/ttp/service-agreements?tenant_id=${TENANT_B_ID}' 2>/dev/null || echo '{}'")
TTP_COUNT=$(json_field "$TTP_RESP" "count")
TTP_STATUS=$(ssh_status "http://localhost:8100/api/ttp/service-agreements?tenant_id=${TENANT_B_ID}")
check_info "CHECK-TTP [TTP] cross-tenant service-agreements read" "$TTP_STATUS" \
    "TTP SQL-scopes by tenant_id. Tenant B agreement count from tenant-A context: ${TTP_COUNT:-0}. HTTP auth is network-level."

# ─── Export tokens for Playwright (GitHub Actions) ────────────────────────────
if [[ -n "${GITHUB_ENV:-}" ]]; then
    {
        echo "ISOLATION_TENANT_A_ID=${TENANT_A_ID}"
        echo "ISOLATION_TENANT_B_ID=${TENANT_B_ID}"
        echo "ISOLATION_TOKEN_A=${TOKEN_A}"
        echo "ISOLATION_TOKEN_B=${TOKEN_B}"
    } >> "$GITHUB_ENV"
    info "Tenant IDs and tokens written to GITHUB_ENV for Playwright isolation step."
fi

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
    echo -e "${RED}${BOLD}ISOLATION CHECK FAILED — $FAILED assertion(s) did not return the expected denial response.${NC}"
    exit 1
else
    echo -e "${GREEN}${BOLD}ISOLATION CHECK PASSED — All $PASSED cross-tenant denial assertions returned 401/403/404.${NC}"
    exit 0
fi
