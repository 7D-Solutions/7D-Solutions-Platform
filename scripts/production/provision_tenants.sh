#!/usr/bin/env bash
# provision_tenants.sh — Bootstrap first real production tenants and platform admin.
#
# Runs entirely via SSH against localhost on the VPS — service ports are firewalled.
# Uses only supported API flows: no direct DB edits for tenant provisioning.
# Platform admin RBAC setup does use the documented seed procedure (seed-platform-admin.sh)
# which runs the auth HTTP API for registration and SQL only for RBAC binding.
#
# What this script does:
#   1. Validates that the plan catalog (cp_plans) contains active plans.
#   2. Provisions 1-2 named production tenants via POST /api/control/tenants.
#   3. Validates each tenant appears in GET /api/tenants and GET /api/control/tenants/:id/summary.
#   4. Optionally bootstraps the platform admin account via scripts/seed-platform-admin.sh.
#   5. Prints a summary of all provisioned resources.
#
# All provisioning calls use idempotency_key values that are stable and deterministic
# (prefixed "prod-initial-v1") so this script is safe to re-run — retries return
# the existing tenant records without creating duplicates.
#
# Usage:
#   PROD_HOST=<host> bash scripts/production/provision_tenants.sh [options]
#
# Required:
#   PROD_HOST — VPS hostname or IP
#
# Optional env:
#   PROD_USER               SSH deploy user   (default: deploy)
#   PROD_SSH_PORT           SSH port          (default: 22)
#   PROD_REPO_PATH          Repo path on VPS  (default: /opt/7d-platform)
#   ADMIN_EMAIL             Platform admin email  (default: admin@7dsolutions.app)
#   ADMIN_PASSWORD          Platform admin password (required if --with-admin)
#   PROVISION_DRY_RUN       Set to 1 for dry-run (prints commands, no SSH calls)
#
# Options:
#   --with-admin            Also bootstrap the platform admin account
#   --dry-run               Print planned actions without executing
#   --host HOST             Override PROD_HOST
#   --user USER             Override PROD_USER
#   --ssh-port PORT         Override PROD_SSH_PORT
#   --repo-path PATH        Override PROD_REPO_PATH

set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────────────────
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

PASSED=0
FAILED=0

# ── Configuration ──────────────────────────────────────────────────────────────
HOST="${PROD_HOST:-}"
SSH_USER="${PROD_USER:-deploy}"
SSH_PORT="${PROD_SSH_PORT:-22}"
REPO_PATH="${PROD_REPO_PATH:-/opt/7d-platform}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@7dsolutions.app}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
WITH_ADMIN=false
DRY_RUN="${PROVISION_DRY_RUN:-0}"

# ── Parse CLI args ─────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --with-admin)       WITH_ADMIN=true;         shift ;;
        --dry-run)          DRY_RUN=1;               shift ;;
        --host)             HOST="$2";               shift 2 ;;
        --user)             SSH_USER="$2";           shift 2 ;;
        --ssh-port)         SSH_PORT="$2";           shift 2 ;;
        --repo-path)        REPO_PATH="$2";          shift 2 ;;
        --admin-email)      ADMIN_EMAIL="$2";        shift 2 ;;
        --admin-password)   ADMIN_PASSWORD="$2";     shift 2 ;;
        --help|-h)
            sed -n '/^# Usage:/,/^[^#]/p' "$0" | grep '^#' | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "ERROR: Unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$HOST" ]]; then
    echo "ERROR: PROD_HOST must be set (or use --host)." >&2
    echo "       Copy scripts/production/env.example → scripts/production/.env.production" >&2
    exit 1
fi

if [[ "$WITH_ADMIN" == true && -z "$ADMIN_PASSWORD" ]]; then
    echo "ERROR: --with-admin requires ADMIN_PASSWORD to be set." >&2
    echo "       Set ADMIN_PASSWORD=<secure-password> before running." >&2
    exit 1
fi

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${SSH_PORT}"
SSH_TARGET="${SSH_USER}@${HOST}"

# ── Helpers ────────────────────────────────────────────────────────────────────
json_field() {
    echo "$1" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$2',''))" 2>/dev/null || true
}

# Run a command on the VPS via SSH (or echo in dry-run mode).
ssh_run() {
    local cmd="$1"
    if [[ "$DRY_RUN" == "1" ]]; then
        echo "[DRY-RUN] ssh ${SSH_TARGET}: ${cmd}" >&2
        echo '{}'
        return
    fi
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" "$cmd"
}

# ── Step 0: Connectivity ───────────────────────────────────────────────────────
banner "Preflight"
info "Production VPS: ${SSH_TARGET} (port ${SSH_PORT})"
info "Repo path on VPS: ${REPO_PATH}"

if [[ "$DRY_RUN" != "1" ]]; then
    if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
        echo "ERROR: Cannot reach ${SSH_TARGET} via SSH." >&2
        exit 1
    fi
    info "SSH connectivity confirmed"
fi

# ── Step 1: Verify plan catalog has active plans ───────────────────────────────
banner "Plan Catalog Check (GET /api/ttp/plans?status=active)"

PLANS_RESP=$(ssh_run "curl -s --max-time 10 'http://localhost:8091/api/ttp/plans?status=active' 2>/dev/null || echo '{}'")

if [[ "$DRY_RUN" == "1" ]]; then
    info "Dry-run: skipping plan catalog check"
    PLAN_TOTAL=3
else
    PLAN_TOTAL=$(echo "$PLANS_RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('total',0))" 2>/dev/null || echo "0")
fi

if [[ "$PLAN_TOTAL" -ge 1 ]]; then
    pass "Plan catalog has ${PLAN_TOTAL} active plan(s)"
else
    fail "Plan catalog is empty — ensure cp_plans migration has run and plans are seeded"
    echo "  Hint: run 'docker compose exec control-plane sqlx migrate run' on the VPS to apply migrations."
    exit 1
fi

# ── Step 2: Provision Tenant A (Starter plan) ─────────────────────────────────
banner "Provision Tenant A — starter/monthly"

TENANT_A_KEY="prod-initial-tenant-a-starter-v1"
TENANT_A_RESP=$(ssh_run "curl -s -X POST 'http://localhost:8091/api/control/tenants' \
    -H 'Content-Type: application/json' \
    --data-raw '{
        \"idempotency_key\": \"${TENANT_A_KEY}\",
        \"environment\": \"production\",
        \"product_code\": \"starter\",
        \"plan_code\": \"monthly\",
        \"concurrent_user_limit\": 10
    }' \
    --max-time 15 2>/dev/null || echo '{}'")

if [[ "$DRY_RUN" == "1" ]]; then
    TENANT_A_ID="00000000-0000-0000-0000-aaaaaaaaaaaa"
    TENANT_A_APP_ID="app-aaaaaaaaaaaa"
    info "Dry-run: Tenant A simulated (${TENANT_A_ID})"
else
    TENANT_A_ID=$(json_field "$TENANT_A_RESP" "tenant_id")
    TENANT_A_APP_ID=$(json_field "$TENANT_A_RESP" "app_id")
    TENANT_A_STATUS=$(json_field "$TENANT_A_RESP" "status")

    if [[ -z "$TENANT_A_ID" ]]; then
        fail "Tenant A provisioning failed — no tenant_id in response"
        echo "  Response: ${TENANT_A_RESP}" >&2
        exit 1
    fi

    pass "Tenant A provisioned — id=${TENANT_A_ID} app_id=${TENANT_A_APP_ID} status=${TENANT_A_STATUS}"
fi

# ── Step 3: Provision Tenant B (Professional plan) ────────────────────────────
banner "Provision Tenant B — professional/monthly"

TENANT_B_KEY="prod-initial-tenant-b-professional-v1"
TENANT_B_RESP=$(ssh_run "curl -s -X POST 'http://localhost:8091/api/control/tenants' \
    -H 'Content-Type: application/json' \
    --data-raw '{
        \"idempotency_key\": \"${TENANT_B_KEY}\",
        \"environment\": \"production\",
        \"product_code\": \"professional\",
        \"plan_code\": \"monthly\",
        \"concurrent_user_limit\": 25
    }' \
    --max-time 15 2>/dev/null || echo '{}'")

if [[ "$DRY_RUN" == "1" ]]; then
    TENANT_B_ID="00000000-0000-0000-0000-bbbbbbbbbbbb"
    TENANT_B_APP_ID="app-bbbbbbbbbbbb"
    info "Dry-run: Tenant B simulated (${TENANT_B_ID})"
else
    TENANT_B_ID=$(json_field "$TENANT_B_RESP" "tenant_id")
    TENANT_B_APP_ID=$(json_field "$TENANT_B_RESP" "app_id")
    TENANT_B_STATUS=$(json_field "$TENANT_B_RESP" "status")

    if [[ -z "$TENANT_B_ID" ]]; then
        fail "Tenant B provisioning failed — no tenant_id in response"
        echo "  Response: ${TENANT_B_RESP}" >&2
        exit 1
    fi

    pass "Tenant B provisioned — id=${TENANT_B_ID} app_id=${TENANT_B_APP_ID} status=${TENANT_B_STATUS}"
fi

# ── Step 4: Validate tenant list contains both tenants ────────────────────────
banner "Validate Tenant List (GET /api/tenants)"

if [[ "$DRY_RUN" == "1" ]]; then
    info "Dry-run: skipping tenant list validation"
else
    TENANT_LIST_RESP=$(ssh_run "curl -s --max-time 10 'http://localhost:8091/api/tenants?page=1&page_size=50' 2>/dev/null || echo '{}'")
    TENANT_LIST_JSON="$TENANT_LIST_RESP"

    FOUND_A=$(echo "$TENANT_LIST_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
tenants = data.get('tenants', data.get('items', []))
ids = [str(t.get('tenant_id','')) for t in tenants]
print('yes' if '$TENANT_A_ID' in ids else 'no')
" 2>/dev/null || echo "error")

    FOUND_B=$(echo "$TENANT_LIST_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
tenants = data.get('tenants', data.get('items', []))
ids = [str(t.get('tenant_id','')) for t in tenants]
print('yes' if '$TENANT_B_ID' in ids else 'no')
" 2>/dev/null || echo "error")

    if [[ "$FOUND_A" == "yes" ]]; then
        pass "Tenant A visible in tenant list"
    else
        fail "Tenant A NOT found in tenant list (id=${TENANT_A_ID})"
    fi

    if [[ "$FOUND_B" == "yes" ]]; then
        pass "Tenant B visible in tenant list"
    else
        fail "Tenant B NOT found in tenant list (id=${TENANT_B_ID})"
    fi
fi

# ── Step 5: Validate tenant summary for each tenant ───────────────────────────
banner "Validate Tenant Summaries"

for pair in "A:${TENANT_A_ID}" "B:${TENANT_B_ID}"; do
    label="${pair%%:*}"
    tid="${pair##*:}"
    if [[ "$DRY_RUN" == "1" ]]; then
        info "Dry-run: skipping summary check for Tenant ${label}"
        continue
    fi
    STATUS_CODE=$(ssh_run "curl -s -o /dev/null -w '%{http_code}' --max-time 10 'http://localhost:8091/api/control/tenants/${tid}/summary' 2>/dev/null || echo 000")
    if [[ "$STATUS_CODE" == "200" ]]; then
        pass "Tenant ${label} summary — HTTP 200"
    else
        fail "Tenant ${label} summary — HTTP ${STATUS_CODE} (expected 200)"
    fi
done

# ── Step 6: Bootstrap platform admin (optional) ───────────────────────────────
if [[ "$WITH_ADMIN" == true ]]; then
    banner "Platform Admin Bootstrap"
    info "Email: ${ADMIN_EMAIL}"
    info "Using seed-platform-admin.sh on VPS at ${REPO_PATH}"

    if [[ "$DRY_RUN" == "1" ]]; then
        info "Dry-run: would run seed-platform-admin.sh on VPS"
    else
        # Run the seed script on the VPS — it uses the auth HTTP API for registration
        # and SQL for RBAC binding (the documented secure bootstrap flow).
        ADMIN_RESULT=$(ssh_run "bash ${REPO_PATH}/scripts/seed-platform-admin.sh \
            --email '${ADMIN_EMAIL}' \
            --password '${ADMIN_PASSWORD}' 2>&1 || true")

        if echo "$ADMIN_RESULT" | grep -q "Platform admin.*is ready\|User registered successfully\|already registered"; then
            pass "Platform admin '${ADMIN_EMAIL}' bootstrapped successfully"
        elif echo "$ADMIN_RESULT" | grep -q "already exists\|already registered"; then
            pass "Platform admin '${ADMIN_EMAIL}' already exists — no action needed"
        else
            fail "Platform admin bootstrap may have failed — check output:"
            echo "$ADMIN_RESULT" | head -20 >&2
        fi
    fi
fi

# ── Step 7: Final plan catalog confirmation ────────────────────────────────────
banner "Final Plan Catalog Confirmation (GET /api/ttp/plans)"

if [[ "$DRY_RUN" != "1" ]]; then
    FINAL_PLANS=$(ssh_run "curl -s --max-time 10 'http://localhost:8091/api/ttp/plans' 2>/dev/null || echo '{}'")
    FINAL_COUNT=$(echo "$FINAL_PLANS" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('total',0))" 2>/dev/null || echo "0")
    PLAN_CODES=$(echo "$FINAL_PLANS" | python3 -c "
import sys, json
d = json.load(sys.stdin)
codes = [p.get('id','') for p in d.get('plans', [])]
print(', '.join(codes))
" 2>/dev/null || echo "unknown")

    if [[ "$FINAL_COUNT" -ge 1 ]]; then
        pass "Plan catalog confirmed: ${FINAL_COUNT} plan(s) — [${PLAN_CODES}]"
    else
        fail "Plan catalog returned 0 plans after provisioning"
    fi
fi

# ── Summary ────────────────────────────────────────────────────────────────────
banner "Provisioning Summary"

echo ""
echo -e "${BOLD}Tenant A:${NC} ${TENANT_A_ID} (starter/monthly, app_id=${TENANT_A_APP_ID})"
echo -e "${BOLD}Tenant B:${NC} ${TENANT_B_ID} (professional/monthly, app_id=${TENANT_B_APP_ID})"
if [[ "$WITH_ADMIN" == true ]]; then
    echo -e "${BOLD}Admin:${NC}    ${ADMIN_EMAIL} (platform_admin role)"
fi
echo ""
echo -e "${BOLD}Checks:${NC} ${GREEN}${PASSED} PASS${NC} / ${RED}${FAILED} FAIL${NC}"
echo ""

if [[ "$FAILED" -gt 0 ]]; then
    echo -e "${RED}${BOLD}PROVISIONING FAILED — ${FAILED} check(s) did not pass.${NC}"
    echo "Run scripts/production/smoke.sh to check overall service health."
    exit 1
fi

echo -e "${GREEN}${BOLD}PROVISIONING COMPLETE.${NC}"
echo ""
echo "Next steps:"
echo "  1. Run smoke.sh to confirm all services are healthy:"
echo "     bash scripts/production/smoke.sh --host ${HOST}"
echo ""
echo "  2. Run isolation_check.sh to confirm tenant isolation:"
echo "     PROD_HOST=${HOST} bash scripts/production/isolation_check.sh"
echo ""
if [[ "$WITH_ADMIN" != true ]]; then
    echo "  3. Bootstrap the platform admin account:"
    echo "     ADMIN_PASSWORD=<secure-password> bash scripts/production/provision_tenants.sh \\"
    echo "       --host ${HOST} --with-admin --admin-email ${ADMIN_EMAIL}"
    echo ""
fi
echo "  Record tenant IDs in your operator runbook for reference."
