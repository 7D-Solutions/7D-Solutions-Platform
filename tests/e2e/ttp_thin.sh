#!/usr/bin/env bash
# TAGS: ttp phase42
# E2E: TTP thin coverage — service agreements (plans) read path + metering trace.
#
# Tests:
#   1. Service agreements list for new tenant → valid shape, empty items
#   2. Service agreements with status filter → valid shape
#   3. Service agreements invalid status → 400
#   4. Metering: ingest 2 events → ingested=2, duplicates=0
#   5. Metering: trace for period → deterministic line items present
#   6. Metering: ingest same events again → duplicates=2, ingested=0
#   7. Metering: trace after duplicate ingest → same result (idempotent)
#   8. Metering: invalid period → 400
#   9. Metering: empty events array → 400
#  10. Metering: trace for period with no events → empty line_items, total=0

# Helpers are sourced by the runner (scripts/e2e_run.sh).

TTP_PORT=$(resolve_port ttp)

echo "=== TTP Thin E2E ==="
echo "[ttp] port $TTP_PORT"

# Wait for service readiness
if ! wait_for_ready "ttp" "$TTP_PORT" "${E2E_TIMEOUT:-30}"; then
    e2e_skip "ttp service not ready — skipping thin tests"
    return 0 2>/dev/null || true
fi

BASE="http://localhost:${TTP_PORT}"

# Generate a unique tenant_id for this test run (UUID v4 via /dev/urandom)
TENANT_ID=$(cat /proc/sys/kernel/random/uuid 2>/dev/null \
    || python3 -c "import uuid; print(uuid.uuid4())" 2>/dev/null \
    || uuidgen 2>/dev/null \
    || echo "00000000-0000-0000-0000-$(date +%s | xxd -p | tail -c 13)")

# Test period — fixed, deterministic
TEST_PERIOD="2026-01"
# Fixed idempotency keys so tests are reproducible across runs with same tenant_id
EVT_KEY_1="e2e-ttp-evt-${TENANT_ID}-001"
EVT_KEY_2="e2e-ttp-evt-${TENANT_ID}-002"

# ============================================================================
# Helper functions
# ============================================================================

http_post() {
    local url="$1" body="$2"
    curl -sf -w '\n%{http_code}' -X POST "$url" \
        -H "Content-Type: application/json" \
        -d "$body" 2>/dev/null
}

http_get() {
    local url="$1"
    curl -sf -w '\n%{http_code}' "$url" 2>/dev/null
}

# Returns body without trailing status line
parse_body() { echo "$1" | sed '$d'; }
parse_status() { echo "$1" | tail -1; }

extract_field() {
    local json="$1" field="$2"
    echo "$json" | grep -o "\"${field}\":[^,}]*" | head -1 | cut -d: -f2- | tr -d ' "'
}

# ============================================================================
# Test 1: Service agreements list — new tenant, expect empty items
# ============================================================================
echo ""
echo "--- Test 1: Service agreements list — empty for new tenant ---"
RAW=$(http_get "${BASE}/api/ttp/service-agreements?tenant_id=${TENANT_ID}")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    # Must have tenant_id, items, count fields
    if echo "$BODY" | grep -q '"tenant_id"' && echo "$BODY" | grep -q '"items"' && echo "$BODY" | grep -q '"count"'; then
        COUNT=$(extract_field "$BODY" "count")
        if [[ "$COUNT" == "0" ]]; then
            e2e_pass "service-agreements list: empty for new tenant (count=0)"
        else
            e2e_fail "service-agreements list: expected count=0 for new tenant, got $COUNT"
        fi
    else
        e2e_fail "service-agreements list: missing required fields — body: $BODY"
    fi
else
    e2e_fail "service-agreements list: HTTP $STATUS (expected 200)"
fi

# ============================================================================
# Test 2: Service agreements with explicit status=all filter
# ============================================================================
echo ""
echo "--- Test 2: Service agreements with status=all ---"
RAW=$(http_get "${BASE}/api/ttp/service-agreements?tenant_id=${TENANT_ID}&status=all")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    if echo "$BODY" | grep -q '"items"' && echo "$BODY" | grep -q '"count"'; then
        e2e_pass "service-agreements status=all: valid shape"
    else
        e2e_fail "service-agreements status=all: missing fields — body: $BODY"
    fi
else
    e2e_fail "service-agreements status=all: HTTP $STATUS (expected 200)"
fi

# ============================================================================
# Test 3: Service agreements — invalid status value → 400
# ============================================================================
echo ""
echo "--- Test 3: Service agreements — invalid status → 400 ---"
RAW=$(curl -s -w '\n%{http_code}' \
    "${BASE}/api/ttp/service-agreements?tenant_id=${TENANT_ID}&status=bogus" 2>/dev/null)
STATUS=$(parse_status "$RAW")

if [[ "$STATUS" == "400" ]]; then
    e2e_pass "service-agreements invalid status: 400"
else
    e2e_fail "service-agreements invalid status: HTTP $STATUS (expected 400)"
fi

# ============================================================================
# Test 4: Metering — ingest 2 events
# ============================================================================
echo ""
echo "--- Test 4: Metering ingest — 2 events ---"
INGEST_BODY=$(cat <<EOF
{
  "tenant_id": "${TENANT_ID}",
  "events": [
    {
      "dimension": "api_calls",
      "quantity": 100,
      "occurred_at": "2026-01-15T10:00:00Z",
      "idempotency_key": "${EVT_KEY_1}"
    },
    {
      "dimension": "api_calls",
      "quantity": 75,
      "occurred_at": "2026-01-20T14:00:00Z",
      "idempotency_key": "${EVT_KEY_2}"
    }
  ]
}
EOF
)

RAW=$(http_post "${BASE}/api/metering/events" "$INGEST_BODY")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    INGESTED=$(extract_field "$BODY" "ingested")
    DUPES=$(extract_field "$BODY" "duplicates")
    if [[ "$INGESTED" == "2" && "$DUPES" == "0" ]]; then
        e2e_pass "metering ingest: ingested=2 duplicates=0"
    else
        e2e_fail "metering ingest: expected ingested=2 duplicates=0, got ingested=${INGESTED} duplicates=${DUPES}"
    fi
else
    e2e_fail "metering ingest: HTTP $STATUS (expected 200) — body: $BODY"
fi

# ============================================================================
# Test 5: Metering trace — deterministic output for known events
# ============================================================================
echo ""
echo "--- Test 5: Metering trace — deterministic output ---"
RAW=$(http_get "${BASE}/api/metering/trace?tenant_id=${TENANT_ID}&period=${TEST_PERIOD}")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    # Must contain required fields
    if echo "$BODY" | grep -q '"tenant_id"' \
        && echo "$BODY" | grep -q '"period"' \
        && echo "$BODY" | grep -q '"line_items"' \
        && echo "$BODY" | grep -q '"total_minor"'; then

        # For our test tenant: 2 api_calls events with qty 100+75=175
        # If no pricing rule, unit_price=0 and total=0; line item still present
        if echo "$BODY" | grep -q '"api_calls"'; then
            TOTAL=$(extract_field "$BODY" "total_minor")
            e2e_pass "metering trace: line_items present (api_calls dimension found), total=${TOTAL}"
        else
            e2e_fail "metering trace: api_calls dimension not found in line_items — body: $BODY"
        fi
    else
        e2e_fail "metering trace: missing required fields — body: $BODY"
    fi
else
    e2e_fail "metering trace: HTTP $STATUS (expected 200) — body: $BODY"
fi

# Capture trace body for idempotency comparison
TRACE_BODY_1="$BODY"

# ============================================================================
# Test 6: Metering idempotency — re-ingest same events, expect all duplicates
# ============================================================================
echo ""
echo "--- Test 6: Metering idempotency — same events → all duplicates ---"
RAW=$(http_post "${BASE}/api/metering/events" "$INGEST_BODY")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    INGESTED=$(extract_field "$BODY" "ingested")
    DUPES=$(extract_field "$BODY" "duplicates")
    if [[ "$INGESTED" == "0" && "$DUPES" == "2" ]]; then
        e2e_pass "metering idempotency: re-ingest gives ingested=0 duplicates=2"
    else
        e2e_fail "metering idempotency: expected ingested=0 duplicates=2, got ingested=${INGESTED} duplicates=${DUPES}"
    fi
else
    e2e_fail "metering idempotency re-ingest: HTTP $STATUS (expected 200) — body: $BODY"
fi

# ============================================================================
# Test 7: Metering trace — same after duplicate ingest (deterministic)
# ============================================================================
echo ""
echo "--- Test 7: Metering trace — same result after duplicate ingest ---"
RAW=$(http_get "${BASE}/api/metering/trace?tenant_id=${TENANT_ID}&period=${TEST_PERIOD}")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    if [[ "$BODY" == "$TRACE_BODY_1" ]]; then
        e2e_pass "metering trace: identical result after duplicate ingest (deterministic)"
    else
        e2e_fail "metering trace: result changed after duplicate ingest — not deterministic"
    fi
else
    e2e_fail "metering trace (idempotency check): HTTP $STATUS (expected 200)"
fi

# ============================================================================
# Test 8: Metering trace — invalid period → 400
# ============================================================================
echo ""
echo "--- Test 8: Metering trace — invalid period → 400 ---"
RAW=$(curl -s -w '\n%{http_code}' \
    "${BASE}/api/metering/trace?tenant_id=${TENANT_ID}&period=202601" 2>/dev/null)
STATUS=$(parse_status "$RAW")

if [[ "$STATUS" == "400" ]]; then
    e2e_pass "metering trace invalid period: 400"
else
    e2e_fail "metering trace invalid period: HTTP $STATUS (expected 400)"
fi

# ============================================================================
# Test 9: Metering ingest — empty events array → 400
# ============================================================================
echo ""
echo "--- Test 9: Metering ingest — empty events → 400 ---"
RAW=$(curl -s -w '\n%{http_code}' -X POST "${BASE}/api/metering/events" \
    -H "Content-Type: application/json" \
    -d "{\"tenant_id\": \"${TENANT_ID}\", \"events\": []}" 2>/dev/null)
STATUS=$(parse_status "$RAW")

if [[ "$STATUS" == "400" ]]; then
    e2e_pass "metering ingest empty events: 400"
else
    e2e_fail "metering ingest empty events: HTTP $STATUS (expected 400)"
fi

# ============================================================================
# Test 10: Metering trace — no-event tenant returns empty line_items
# ============================================================================
echo ""
echo "--- Test 10: Metering trace — no events for period ---"
# Use a completely different tenant that has no events
EMPTY_TENANT=$(cat /proc/sys/kernel/random/uuid 2>/dev/null \
    || python3 -c "import uuid; print(uuid.uuid4())" 2>/dev/null \
    || uuidgen 2>/dev/null \
    || echo "00000000-0000-0000-0000-$(date +%s%N | tail -c 13 | xxd -p | tail -c 13)")

RAW=$(http_get "${BASE}/api/metering/trace?tenant_id=${EMPTY_TENANT}&period=${TEST_PERIOD}")
STATUS=$(parse_status "$RAW")
BODY=$(parse_body "$RAW")

if [[ "$STATUS" == "200" ]]; then
    TOTAL=$(extract_field "$BODY" "total_minor")
    # total_minor should be 0 with no events
    if [[ "$TOTAL" == "0" ]]; then
        e2e_pass "metering trace empty tenant: total_minor=0"
    else
        e2e_fail "metering trace empty tenant: expected total_minor=0, got ${TOTAL}"
    fi
else
    e2e_fail "metering trace empty tenant: HTTP $STATUS (expected 200) — body: $BODY"
fi

echo ""
echo "=== TTP Thin E2E Complete ==="
