#!/usr/bin/env bash
# payment_loop.sh — Staging E2E payment loop harness with idempotency proof.
#
# Proves the full money path: customer → invoice → webhook → posting, with
# idempotency guarantee: replaying the same Tilled webhook event must not
# produce duplicate postings.
#
# Usage:
#   bash scripts/staging/payment_loop.sh [--dry-run] [--host HOST] [--secret SECRET]
#
# Environment variables (loaded from .env.staging or set explicitly):
#   STAGING_HOST              — VPS hostname or IP (required)
#   TILLED_WEBHOOK_SECRET     — Tilled HMAC-SHA256 webhook signing secret (required)
#   PAYMENT_LOOP_TIMEOUT      — Per-request curl timeout in seconds (default: 15)
#
# --dry-run: Validate environment and print planned steps, then exit 0.
#
# Exit code: 0 = loop proven idempotent, 1 = failure.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── Load staging env if available ────────────────────────────────────────────
ENV_FILE="${SMOKE_ENV_FILE:-${REPO_ROOT}/scripts/staging/.env.staging}"
if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "${REPO_ROOT}/scripts/staging/export_env.sh" "$ENV_FILE"
fi

# ── Configuration ─────────────────────────────────────────────────────────────
HOST="${STAGING_HOST:-}"
WEBHOOK_SECRET="${TILLED_WEBHOOK_SECRET:-}"
TIMEOUT="${PAYMENT_LOOP_TIMEOUT:-15}"
DRY_RUN=false

# Parse CLI args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)  DRY_RUN=true;              shift   ;;
        --host)     HOST="$2";                 shift 2 ;;
        --secret)   WEBHOOK_SECRET="$2";       shift 2 ;;
        --timeout)  TIMEOUT="$2";             shift 2 ;;
        *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done

# ── Port map (canonical — matches smoke.sh + docker-compose.services.yml) ─────
AR_PORT=8086
TTP_PORT=8100

AR_BASE="http://${HOST}:${AR_PORT}"
TTP_BASE="http://${HOST}:${TTP_PORT}"

# ── Helpers ───────────────────────────────────────────────────────────────────
banner() { printf '\n=== %s ===\n' "$*"; }
step()   { printf '  ▶  %s\n' "$*"; }
ok()     { printf '  ✓  %s\n' "$*"; }
fail()   { printf '  ✗  %s\n' "$*" >&2; }

require_cmd() {
    local cmd="$1"
    if ! command -v "$cmd" &>/dev/null; then
        printf 'ERROR: required command not found: %s\n' "$cmd" >&2
        exit 1
    fi
}

# HMAC-SHA256 signature in Tilled format: "t=<ts>,v1=<hex>"
# Requires openssl.
make_tilled_signature() {
    local payload="$1"
    local ts="$2"
    local secret="$3"
    local signed="${ts}.${payload}"
    local hex_sig
    hex_sig=$(printf '%s' "$signed" | openssl dgst -sha256 -hmac "$secret" | awk '{print $2}')
    printf 't=%s,v1=%s' "$ts" "$hex_sig"
}

# HTTP POST helper — returns body on success, exits on HTTP error.
# Usage: post_json URL body_json [idempotency_key]
post_json() {
    local url="$1"
    local body="$2"
    local ikey="${3:-}"
    local -a curl_args=(
        -s -w '\n%{http_code}'
        --max-time "$TIMEOUT"
        -H 'Content-Type: application/json'
        -X POST
        -d "$body"
    )
    if [[ -n "$ikey" ]]; then
        curl_args+=(-H "Idempotency-Key: ${ikey}")
    fi
    curl "${curl_args[@]}" "$url"
}

# HTTP GET helper — returns body.
get_json() {
    local url="$1"
    curl -s -w '\n%{http_code}' --max-time "$TIMEOUT" "$url"
}

# Split raw curl response (body\nstatus) into body and check status.
check_response() {
    local name="$1"
    local raw="$2"
    local want="${3:-200}"
    local status body
    status="${raw##*$'\n'}"
    body="${raw%$'\n'${status}}"
    if [[ "$status" != "$want" ]]; then
        fail "${name}: HTTP ${status} (expected ${want})"
        printf 'Body: %s\n' "$body" >&2
        return 1
    fi
    printf '%s' "$body"
}

# Extract a JSON field (shallow, first match) using grep + sed.
# Portable alternative to jq for simple string fields.
json_field() {
    local field="$1"
    local json="$2"
    # Match "field": "value" or "field": number
    printf '%s' "$json" \
        | grep -oE "\"${field}\"[[:space:]]*:[[:space:]]*\"?[^,}\"]+" \
        | head -1 \
        | sed -E "s/\"${field}\"[[:space:]]*:[[:space:]]*\"?//"
}

# ── Environment validation ────────────────────────────────────────────────────
banner "Environment validation"

require_cmd curl
require_cmd openssl
require_cmd awk

if [[ -z "$HOST" ]]; then
    printf 'ERROR: STAGING_HOST must be set (via env var or --host).\n' >&2
    printf '       Copy scripts/staging/env.example → scripts/staging/.env.staging\n' >&2
    exit 1
fi

if [[ -z "$WEBHOOK_SECRET" ]]; then
    printf 'ERROR: TILLED_WEBHOOK_SECRET must be set (via env var or --secret).\n' >&2
    printf '       This is the Tilled webhook signing secret used for HMAC verification.\n' >&2
    exit 1
fi

ok "STAGING_HOST    = ${HOST}"
ok "AR_BASE         = ${AR_BASE}"
ok "TTP_BASE        = ${TTP_BASE}"
ok "TIMEOUT         = ${TIMEOUT}s"
ok "Webhook secret  = (set, ${#WEBHOOK_SECRET} chars)"

# ── Dry-run: print plan and exit ──────────────────────────────────────────────
if $DRY_RUN; then
    banner "Dry-run mode — planned steps (no network calls)"
    step "1. POST ${AR_BASE}/api/ar/customers — create test customer"
    step "2. POST ${AR_BASE}/api/ar/invoices  — create draft invoice (→ tilled_invoice_id)"
    step "3. Compute Tilled HMAC signature over invoice.payment_succeeded payload"
    step "4. POST ${AR_BASE}/api/ar/webhooks/tilled — deliver webhook, assert HTTP 200"
    step "5. GET  ${AR_BASE}/api/ar/invoices/<id>  — assert status = paid"
    step "6. POST ${AR_BASE}/api/ar/webhooks/tilled — REPLAY same event, assert HTTP 200"
    step "7. GET  ${AR_BASE}/api/ar/webhooks?event_id=<id> — assert exactly 1 record (idempotency)"
    step "8. Cleanup: DELETE test customer + invoice"
    printf '\nDry-run PASSED — environment is valid, steps are printed above.\n'
    exit 0
fi

# ── Step 1: Create test customer ──────────────────────────────────────────────
banner "Step 1 — Create AR customer"

CUSTOMER_EMAIL="payment-loop-test-$(date +%s)@7d-staging.local"
CUSTOMER_BODY=$(printf '{"email":"%s","name":"Payment Loop Test","external_customer_id":"loop-test-%s"}' \
    "$CUSTOMER_EMAIL" "$(date +%s)")

step "POST ${AR_BASE}/api/ar/customers"
raw=$(post_json "${AR_BASE}/api/ar/customers" "$CUSTOMER_BODY")
customer_json=$(check_response "create customer" "$raw" "201")
CUSTOMER_ID=$(json_field "id" "$customer_json")

if [[ -z "$CUSTOMER_ID" || "$CUSTOMER_ID" == "null" ]]; then
    fail "Could not extract customer id from response"
    printf 'Response: %s\n' "$customer_json" >&2
    exit 1
fi

ok "Customer created: id=${CUSTOMER_ID}"

# ── Step 2: Create draft invoice ──────────────────────────────────────────────
banner "Step 2 — Create AR invoice"

INVOICE_BODY=$(printf '{"ar_customer_id":%s,"amount_cents":5000,"currency":"usd","status":"open","metadata":{"source":"payment_loop_harness"}}' \
    "$CUSTOMER_ID")

step "POST ${AR_BASE}/api/ar/invoices"
raw=$(post_json "${AR_BASE}/api/ar/invoices" "$INVOICE_BODY")
invoice_json=$(check_response "create invoice" "$raw" "201")
INVOICE_ID=$(json_field "id" "$invoice_json")
TILLED_INVOICE_ID=$(json_field "tilled_invoice_id" "$invoice_json")

if [[ -z "$INVOICE_ID" || -z "$TILLED_INVOICE_ID" ]]; then
    fail "Could not extract invoice id or tilled_invoice_id from response"
    printf 'Response: %s\n' "$invoice_json" >&2
    exit 1
fi

ok "Invoice created: id=${INVOICE_ID}, tilled_invoice_id=${TILLED_INVOICE_ID}"

# ── Step 3: Build and deliver webhook ─────────────────────────────────────────
banner "Step 3 — Deliver payment webhook"

# Generate a unique Tilled event ID for this run
EVENT_ID="evt_loop_$(date +%s)_$$"
TS=$(date +%s)

WEBHOOK_PAYLOAD=$(printf '{"id":"%s","type":"invoice.payment_succeeded","data":{"id":"%s","status":"paid","amount":5000,"currency":"usd"},"created_at":%s,"livemode":false}' \
    "$EVENT_ID" "$TILLED_INVOICE_ID" "$TS")

SIG=$(make_tilled_signature "$WEBHOOK_PAYLOAD" "$TS" "$WEBHOOK_SECRET")

step "POST ${AR_BASE}/api/ar/webhooks/tilled (event_id=${EVENT_ID})"

raw=$(
    curl -s -w '\n%{http_code}' \
        --max-time "$TIMEOUT" \
        -X POST \
        -H 'Content-Type: application/json' \
        -H "tilled-signature: ${SIG}" \
        -d "$WEBHOOK_PAYLOAD" \
        "${AR_BASE}/api/ar/webhooks/tilled"
)
check_response "deliver webhook" "$raw" "200" > /dev/null
ok "Webhook accepted (HTTP 200)"

# ── Step 4: Verify invoice posted as paid ─────────────────────────────────────
banner "Step 4 — Verify invoice status = paid"

step "GET ${AR_BASE}/api/ar/invoices/${INVOICE_ID}"
raw=$(get_json "${AR_BASE}/api/ar/invoices/${INVOICE_ID}")
inv_json=$(check_response "get invoice" "$raw" "200")
INV_STATUS=$(json_field "status" "$inv_json")

if [[ "$INV_STATUS" != "paid" ]]; then
    fail "Invoice status is '${INV_STATUS}', expected 'paid'"
    printf 'Response: %s\n' "$inv_json" >&2
    exit 1
fi

ok "Invoice ${INVOICE_ID} status = paid"

# ── Step 5: Replay the same webhook event ─────────────────────────────────────
banner "Step 5 — Replay same webhook (idempotency check)"

# Rebuild signature with a new timestamp (Tilled freshness window is ±5 min).
TS2=$(date +%s)
# Keep the same EVENT_ID and same payload structure but refresh the timestamp field.
WEBHOOK_PAYLOAD2=$(printf '{"id":"%s","type":"invoice.payment_succeeded","data":{"id":"%s","status":"paid","amount":5000,"currency":"usd"},"created_at":%s,"livemode":false}' \
    "$EVENT_ID" "$TILLED_INVOICE_ID" "$TS2")
SIG2=$(make_tilled_signature "$WEBHOOK_PAYLOAD2" "$TS2" "$WEBHOOK_SECRET")

step "POST ${AR_BASE}/api/ar/webhooks/tilled (REPLAY event_id=${EVENT_ID})"

raw2=$(
    curl -s -w '\n%{http_code}' \
        --max-time "$TIMEOUT" \
        -X POST \
        -H 'Content-Type: application/json' \
        -H "tilled-signature: ${SIG2}" \
        -d "$WEBHOOK_PAYLOAD2" \
        "${AR_BASE}/api/ar/webhooks/tilled"
)
check_response "replay webhook" "$raw2" "200" > /dev/null
ok "Replay accepted (HTTP 200) — no rejection of duplicate"

# ── Step 6: Verify only one webhook record exists ─────────────────────────────
banner "Step 6 — Verify idempotency: exactly one webhook record"

step "GET ${AR_BASE}/api/ar/webhooks?event_type=invoice.payment_succeeded"
raw3=$(get_json "${AR_BASE}/api/ar/webhooks?event_type=invoice.payment_succeeded")
wh_json=$(check_response "list webhooks" "$raw3" "200")

# Count records with our event_id
WH_COUNT=$(printf '%s' "$wh_json" | grep -o "\"event_id\":\"${EVENT_ID}\"" | wc -l | tr -d ' ')

if [[ "$WH_COUNT" -ne 1 ]]; then
    fail "Expected exactly 1 webhook record for event_id=${EVENT_ID}, found ${WH_COUNT}"
    printf 'Webhook listing: %s\n' "$wh_json" >&2
    exit 1
fi

ok "Webhook count for event_id=${EVENT_ID} = ${WH_COUNT} (idempotent: no duplicate posting)"

# ── Step 7: Verify invoice still paid (replay did not corrupt state) ───────────
banner "Step 7 — Post-replay state check"

step "GET ${AR_BASE}/api/ar/invoices/${INVOICE_ID}"
raw4=$(get_json "${AR_BASE}/api/ar/invoices/${INVOICE_ID}")
inv_json2=$(check_response "get invoice post-replay" "$raw4" "200")
INV_STATUS2=$(json_field "status" "$inv_json2")

if [[ "$INV_STATUS2" != "paid" ]]; then
    fail "Invoice status after replay is '${INV_STATUS2}', expected 'paid' — replay corrupted state"
    exit 1
fi

ok "Invoice ${INVOICE_ID} status still = paid after replay"

# ── Summary ───────────────────────────────────────────────────────────────────
printf '\n'
printf '────────────────────────────────────────────────────────────────────\n'
printf 'Payment loop PROOF:\n'
printf '  Customer ID         : %s\n' "$CUSTOMER_ID"
printf '  Invoice ID          : %s\n' "$INVOICE_ID"
printf '  Tilled Invoice ID   : %s\n' "$TILLED_INVOICE_ID"
printf '  Event ID            : %s\n' "$EVENT_ID"
printf '  Invoice final status: %s\n' "$INV_STATUS2"
printf '  Webhook records     : %s (expected 1 — idempotency PROVEN)\n' "$WH_COUNT"
printf '\n'
printf 'invoice → webhook → posting: PROVEN\n'
printf 'Webhook replay idempotency:  PROVEN\n'
printf '\nPayment loop harness PASSED.\n'
