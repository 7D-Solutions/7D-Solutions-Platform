#!/usr/bin/env bash
# payment_verify.sh — Production payment path verification (Tilled test mode).
#
# Proves the full money path using Tilled's test-mode webhook events:
#   customer → invoice → webhook → posting + idempotency replay.
#
# Production-safe: uses livemode=false payloads so no real money moves.
# All HTTP calls run from inside the production VPS via SSH (ports are firewalled).
# The HMAC signature is computed on the CI runner — TILLED_WEBHOOK_SECRET never
# touches the VPS process tree.
#
# Usage:
#   PROD_HOST=<host> TILLED_WEBHOOK_SECRET=<secret> bash scripts/production/payment_verify.sh
#   bash scripts/production/payment_verify.sh [--dry-run] [--host HOST] [--secret SECRET]
#
# Environment variables:
#   PROD_HOST               VPS hostname or IP (required)
#   PROD_USER               SSH deploy user (default: deploy)
#   PROD_SSH_PORT           SSH port (default: 22)
#   TILLED_WEBHOOK_SECRET   Tilled HMAC-SHA256 signing secret (required)
#   PAYMENT_VERIFY_TIMEOUT  Per-request curl timeout in seconds (default: 20)
#
# Exit code: 0 = payment path proven, 1 = failure.

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────────────
HOST="${PROD_HOST:-}"
USER="${PROD_USER:-deploy}"
SSH_PORT="${PROD_SSH_PORT:-22}"
WEBHOOK_SECRET="${TILLED_WEBHOOK_SECRET:-}"
TIMEOUT="${PAYMENT_VERIFY_TIMEOUT:-20}"
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)  DRY_RUN=true;            shift   ;;
        --host)     HOST="$2";               shift 2 ;;
        --secret)   WEBHOOK_SECRET="$2";     shift 2 ;;
        --timeout)  TIMEOUT="$2";            shift 2 ;;
        *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${SSH_PORT}"
SSH_TARGET="${USER}@${HOST}"

# ── Helpers ───────────────────────────────────────────────────────────────────
banner() { printf '\n=== %s ===\n' "$*"; }
step()   { printf '  ▶  %s\n' "$*"; }
ok()     { printf '  ✓  %s\n' "$*"; }
fail()   { printf '  ✗  %s\n' "$*" >&2; }

json_field() {
    local field="$1" json="$2"
    printf '%s' "$json" \
        | grep -oE "\"${field}\"[[:space:]]*:[[:space:]]*\"?[^,}\"]+" \
        | head -1 \
        | sed -E "s/\"${field}\"[[:space:]]*:[[:space:]]*\"?//"
}

# Compute Tilled HMAC-SHA256 signature on the CI runner (secret stays local).
# Format: "t=<ts>,v1=<hex>"
make_tilled_sig() {
    local payload="$1" ts="$2" secret="$3"
    local hex
    hex=$(printf '%s' "${ts}.${payload}" | openssl dgst -sha256 -hmac "$secret" | awk '{print $2}')
    printf 't=%s,v1=%s' "$ts" "$hex"
}

# Run curl from inside the VPS via SSH against localhost.
# Usage: ssh_curl_status <url> [extra_curl_flags_as_single_string]
ssh_curl_status() {
    local url="$1" extra="${2:-}"
    if $DRY_RUN; then echo "200"; return; fi
    # shellcheck disable=SC2029,SC2086
    ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -o /dev/null -w '%{http_code}' --max-time ${TIMEOUT} ${extra} '${url}' 2>/dev/null || echo 000"
}

# Run curl from inside the VPS; returns body\nstatus.
ssh_curl_body() {
    local url="$1" extra="${2:-}"
    if $DRY_RUN; then printf '{"id":1,"tilled_invoice_id":"tilled_test_inv_dry"}\n201'; return; fi
    # shellcheck disable=SC2029,SC2086
    ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -w '\n%{http_code}' --max-time ${TIMEOUT} ${extra} '${url}' 2>/dev/null || printf '\n000'"
}

split_response() {
    local raw="$1"
    STATUS="${raw##*$'\n'}"
    BODY="${raw%$'\n'${STATUS}}"
}

# ── Environment validation ─────────────────────────────────────────────────────
banner "Environment validation"

for cmd in openssl awk; do
    if ! command -v "$cmd" &>/dev/null; then
        printf 'ERROR: required command not found: %s\n' "$cmd" >&2; exit 1
    fi
done

if [[ -z "$HOST" ]]; then
    printf 'ERROR: PROD_HOST must be set (via env var or --host).\n' >&2; exit 1
fi
if [[ -z "$WEBHOOK_SECRET" ]]; then
    printf 'ERROR: TILLED_WEBHOOK_SECRET must be set (via env var or --secret).\n' >&2; exit 1
fi

if ! $DRY_RUN; then
    if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
        printf 'ERROR: Cannot reach %s via SSH.\n' "$SSH_TARGET" >&2; exit 1
    fi
fi

ok "PROD_HOST       = ${HOST} (SSH to localhost)"
ok "Webhook secret  = (set, ${#WEBHOOK_SECRET} chars)"
ok "Timeout         = ${TIMEOUT}s"

# ── Dry-run plan ──────────────────────────────────────────────────────────────
if $DRY_RUN; then
    banner "Dry-run mode — planned steps (no network calls)"
    step "1. POST localhost:8086/api/ar/customers  — create test customer"
    step "2. POST localhost:8086/api/ar/invoices   — create draft invoice"
    step "3. Compute Tilled HMAC-SHA256 signature locally (secret stays on CI runner)"
    step "4. POST localhost:8086/api/ar/webhooks/tilled — deliver webhook (livemode=false)"
    step "5. GET  localhost:8086/api/ar/invoices/<id>   — assert status = paid"
    step "6. POST localhost:8086/api/ar/webhooks/tilled — REPLAY same event (idempotency)"
    step "7. GET  localhost:8086/api/ar/webhooks?event_type=invoice.payment_succeeded — assert 1 record"
    printf '\nDry-run PASSED — environment valid.\n'
    exit 0
fi

AR_BASE="http://localhost:8086"

# ── Step 1: Create test customer ──────────────────────────────────────────────
banner "Step 1 — Create AR customer (test fixture)"

RUN_TS="$(date +%s)"
EMAIL="payment-verify-prod-${RUN_TS}@7d-prod-test.internal"
CUSTOMER_JSON="{\"email\":\"${EMAIL}\",\"name\":\"Prod Payment Verify\",\"external_customer_id\":\"prod-verify-${RUN_TS}\"}"

step "POST ${AR_BASE}/api/ar/customers (via SSH localhost)"
RAW=$(ssh_curl_body "${AR_BASE}/api/ar/customers" \
    "-X POST -H 'Content-Type: application/json' -d '${CUSTOMER_JSON}'")
split_response "$RAW"

if [[ "$STATUS" != "201" ]]; then
    fail "Create customer: HTTP ${STATUS} (expected 201)"
    printf 'Body: %s\n' "$BODY" >&2; exit 1
fi
CUSTOMER_ID=$(json_field "id" "$BODY")
[[ -z "$CUSTOMER_ID" || "$CUSTOMER_ID" == "null" ]] && { fail "No customer id in response: ${BODY}" >&2; exit 1; }
ok "Customer created: id=${CUSTOMER_ID}"

# ── Step 2: Create draft invoice ──────────────────────────────────────────────
banner "Step 2 — Create AR invoice"

INVOICE_JSON="{\"ar_customer_id\":${CUSTOMER_ID},\"amount_cents\":1000,\"currency\":\"usd\",\"status\":\"open\",\"metadata\":{\"source\":\"prod_payment_verify\"}}"

step "POST ${AR_BASE}/api/ar/invoices (via SSH localhost)"
RAW=$(ssh_curl_body "${AR_BASE}/api/ar/invoices" \
    "-X POST -H 'Content-Type: application/json' -d '${INVOICE_JSON}'")
split_response "$RAW"

if [[ "$STATUS" != "201" ]]; then
    fail "Create invoice: HTTP ${STATUS} (expected 201)"
    printf 'Body: %s\n' "$BODY" >&2; exit 1
fi
INVOICE_ID=$(json_field "id" "$BODY")
TILLED_ID=$(json_field "tilled_invoice_id" "$BODY")
[[ -z "$INVOICE_ID" || -z "$TILLED_ID" ]] && { fail "Missing invoice id or tilled_invoice_id: ${BODY}" >&2; exit 1; }
ok "Invoice created: id=${INVOICE_ID}, tilled_invoice_id=${TILLED_ID}"

# ── Step 3: Deliver webhook (HMAC signed, test mode) ──────────────────────────
banner "Step 3 — Deliver payment webhook (Tilled test mode, livemode=false)"

EVENT_ID="evt_prod_verify_${RUN_TS}_$$"
TS1=$(date +%s)
PAYLOAD1="{\"id\":\"${EVENT_ID}\",\"type\":\"invoice.payment_succeeded\",\"data\":{\"id\":\"${TILLED_ID}\",\"status\":\"paid\",\"amount\":1000,\"currency\":\"usd\"},\"created_at\":${TS1},\"livemode\":false}"
SIG1=$(make_tilled_sig "$PAYLOAD1" "$TS1" "$WEBHOOK_SECRET")

step "POST ${AR_BASE}/api/ar/webhooks/tilled (event_id=${EVENT_ID}, livemode=false)"
STATUS1=$(ssh_curl_status "${AR_BASE}/api/ar/webhooks/tilled" \
    "-X POST -H 'Content-Type: application/json' -H 'tilled-signature: ${SIG1}' -d '${PAYLOAD1}'")

if [[ "$STATUS1" != "200" ]]; then
    fail "Webhook delivery: HTTP ${STATUS1} (expected 200)"
    exit 1
fi
ok "Webhook accepted (HTTP 200) — livemode=false confirmed"

# ── Step 4: Verify invoice marked paid ────────────────────────────────────────
banner "Step 4 — Verify invoice status = paid"

step "GET ${AR_BASE}/api/ar/invoices/${INVOICE_ID} (via SSH localhost)"
RAW=$(ssh_curl_body "${AR_BASE}/api/ar/invoices/${INVOICE_ID}")
split_response "$RAW"

[[ "$STATUS" != "200" ]] && { fail "GET invoice: HTTP ${STATUS} (expected 200)"; exit 1; }
INV_STATUS=$(json_field "status" "$BODY")
[[ "$INV_STATUS" != "paid" ]] && { fail "Invoice status='${INV_STATUS}' (expected 'paid')"; printf 'Body: %s\n' "$BODY" >&2; exit 1; }
ok "Invoice ${INVOICE_ID} status = paid"

# ── Step 5: Replay same event (idempotency) ────────────────────────────────────
banner "Step 5 — Replay webhook (idempotency proof)"

TS2=$(date +%s)
PAYLOAD2="{\"id\":\"${EVENT_ID}\",\"type\":\"invoice.payment_succeeded\",\"data\":{\"id\":\"${TILLED_ID}\",\"status\":\"paid\",\"amount\":1000,\"currency\":\"usd\"},\"created_at\":${TS2},\"livemode\":false}"
SIG2=$(make_tilled_sig "$PAYLOAD2" "$TS2" "$WEBHOOK_SECRET")

step "POST ${AR_BASE}/api/ar/webhooks/tilled (REPLAY event_id=${EVENT_ID})"
STATUS2=$(ssh_curl_status "${AR_BASE}/api/ar/webhooks/tilled" \
    "-X POST -H 'Content-Type: application/json' -H 'tilled-signature: ${SIG2}' -d '${PAYLOAD2}'")

[[ "$STATUS2" != "200" ]] && { fail "Replay: HTTP ${STATUS2} (expected 200)"; exit 1; }
ok "Replay accepted (HTTP 200)"

# ── Step 6: Verify exactly one webhook record ──────────────────────────────────
banner "Step 6 — Idempotency: assert exactly 1 webhook record"

step "GET ${AR_BASE}/api/ar/webhooks?event_type=invoice.payment_succeeded (via SSH localhost)"
RAW=$(ssh_curl_body "${AR_BASE}/api/ar/webhooks?event_type=invoice.payment_succeeded")
split_response "$RAW"

[[ "$STATUS" != "200" ]] && { fail "List webhooks: HTTP ${STATUS} (expected 200)"; exit 1; }
WH_COUNT=$(printf '%s' "$BODY" | grep -o "\"event_id\":\"${EVENT_ID}\"" | wc -l | tr -d ' ')

if [[ "$WH_COUNT" -ne 1 ]]; then
    fail "Expected 1 webhook record for event_id=${EVENT_ID}, found ${WH_COUNT}"
    printf 'Response: %s\n' "$BODY" >&2; exit 1
fi
ok "Webhook records for event_id=${EVENT_ID}: ${WH_COUNT} (idempotency PROVEN)"

# ── Summary ───────────────────────────────────────────────────────────────────
printf '\n'
printf '────────────────────────────────────────────────────────────\n'
printf 'Production payment verification PROOF:\n'
printf '  Customer ID          : %s\n' "$CUSTOMER_ID"
printf '  Invoice ID           : %s\n' "$INVOICE_ID"
printf '  Tilled Invoice ID    : %s\n' "$TILLED_ID"
printf '  Event ID             : %s\n' "$EVENT_ID"
printf '  livemode             : false (Tilled test mode — no real money moved)\n'
printf '  Invoice final status : paid\n'
printf '  Webhook records      : %s (expected 1 — idempotency PROVEN)\n' "$WH_COUNT"
printf '\n'
printf 'invoice → webhook → posting (test mode): PROVEN\n'
printf 'Webhook replay idempotency:               PROVEN\n'
printf '\nProduction payment verification PASSED.\n'
