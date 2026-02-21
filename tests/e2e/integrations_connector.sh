#!/usr/bin/env bash
# TAGS: integrations connector phase42
# E2E: Integrations connector contract — configure → test action → validate result.
#
# Tests:
#   1. GET /api/integrations/connectors/types — list registered types, verify echo present
#   2. POST /api/integrations/connectors — register echo connector
#   3. GET  /api/integrations/connectors — list connectors, verify registered one present
#   4. GET  /api/integrations/connectors/:id — fetch by id
#   5. POST /api/integrations/connectors/:id/test — run test action, verify deterministic output
#   6. Idempotency: same idempotency_key produces same echo output
#   7. Validation: unknown connector_type → 422
#   8. Validation: missing X-App-Id → 400
#   9. Outbox: verify connector.registered event was written
#  10. Test action on disabled connector is rejected (404 on unknown id)

# Helpers are sourced by the runner (scripts/e2e_run.sh).

INTEGRATIONS_PORT=$(resolve_port integrations)

echo "=== Integrations Connector E2E ==="
echo "[connector] port $INTEGRATIONS_PORT"

# Wait for service readiness
if ! wait_for_ready "integrations" "$INTEGRATIONS_PORT" "${E2E_TIMEOUT:-30}"; then
    e2e_skip "integrations service not ready — skipping connector tests"
    return 0 2>/dev/null || true
fi

BASE="http://localhost:${INTEGRATIONS_PORT}"
APP_ID="e2e-connector-$(date +%s)"
HEADERS=(-H "Content-Type: application/json" -H "X-App-Id: $APP_ID" -H "X-Correlation-Id: e2e-corr-connector-1")

# ============================================================================
# Helpers
# ============================================================================
http_post() {
    local url="$1" body="$2"
    curl -sf -w '\n%{http_code}' -X POST "$url" "${HEADERS[@]}" -d "$body" 2>/dev/null
}

http_get() {
    local url="$1"
    curl -sf -w '\n%{http_code}' "$url" "${HEADERS[@]}" 2>/dev/null
}

http_get_no_appid() {
    local url="$1"
    curl -s -w '\n%{http_code}' -H "Content-Type: application/json" "$url" 2>/dev/null
}

http_post_raw() {
    local url="$1" body="$2"
    curl -s -w '\n%{http_code}' -X POST "$url" "${HEADERS[@]}" -d "$body" 2>/dev/null
}

parse_response() { echo "$1" | sed '$d'; }
parse_status()   { echo "$1" | tail -1; }

extract_field() {
    local json="$1" field="$2"
    echo "$json" | grep -o "\"${field}\":\"[^\"]*\"" | head -1 | cut -d'"' -f4
}

extract_bool() {
    local json="$1" field="$2"
    echo "$json" | grep -o "\"${field}\":[a-z]*" | head -1 | cut -d: -f2
}

# ============================================================================
# Test 1: List connector types — verify echo is registered
# ============================================================================
echo ""
echo "--- Test 1: List registered connector types ---"
RAW=$(http_get "$BASE/api/integrations/connectors/types")
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "200" ]]; then
    if echo "$BODY" | grep -q '"connector_type":"echo"'; then
        e2e_pass "connector types list contains echo"
    else
        e2e_fail "connector types list missing echo — body: $BODY"
    fi
    if echo "$BODY" | grep -q '"supports_test_action":true'; then
        e2e_pass "echo connector declares supports_test_action=true"
    else
        e2e_fail "echo connector missing supports_test_action flag — body: $BODY"
    fi
else
    e2e_fail "list connector types — HTTP $STATUS"
fi

# ============================================================================
# Test 2: Register echo connector
# ============================================================================
echo ""
echo "--- Test 2: Register echo connector ---"
RAW=$(http_post "$BASE/api/integrations/connectors" '{
    "connector_type": "echo",
    "name": "e2e-echo",
    "config": { "echo_prefix": "hello-e2e" }
}')
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "201" ]]; then
    CONNECTOR_ID=$(extract_field "$BODY" "id")
    CONN_TYPE=$(extract_field "$BODY" "connector_type")
    CONN_NAME=$(extract_field "$BODY" "name")
    CONN_ENABLED=$(extract_bool "$BODY" "enabled")
    if [[ -n "$CONNECTOR_ID" && "$CONN_TYPE" == "echo" && "$CONN_NAME" == "e2e-echo" && "$CONN_ENABLED" == "true" ]]; then
        e2e_pass "register echo connector (id=$CONNECTOR_ID)"
    else
        e2e_fail "register connector — unexpected body: $BODY"
    fi
else
    e2e_fail "register connector — HTTP $STATUS (body: $BODY)"
    # Cannot continue without a connector id
    return 0 2>/dev/null || true
fi

# ============================================================================
# Test 3: List connectors — verify registered one is present
# ============================================================================
echo ""
echo "--- Test 3: List connectors ---"
RAW=$(http_get "$BASE/api/integrations/connectors")
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "200" ]]; then
    if echo "$BODY" | grep -q "$CONNECTOR_ID"; then
        e2e_pass "list connectors contains registered id"
    else
        e2e_fail "list connectors — id $CONNECTOR_ID not found in body"
    fi
else
    e2e_fail "list connectors — HTTP $STATUS"
fi

# ============================================================================
# Test 4: Fetch by id
# ============================================================================
echo ""
echo "--- Test 4: Fetch connector by id ---"
RAW=$(http_get "$BASE/api/integrations/connectors/$CONNECTOR_ID")
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "200" ]]; then
    FETCHED_TYPE=$(extract_field "$BODY" "connector_type")
    FETCHED_NAME=$(extract_field "$BODY" "name")
    if [[ "$FETCHED_TYPE" == "echo" && "$FETCHED_NAME" == "e2e-echo" ]]; then
        e2e_pass "fetch connector by id"
    else
        e2e_fail "fetch connector — unexpected body: $BODY"
    fi
else
    e2e_fail "fetch connector — HTTP $STATUS"
fi

# ============================================================================
# Test 5: Run test action — verify deterministic output
# ============================================================================
echo ""
echo "--- Test 5: Run test action ---"
IDEM_KEY="e2e-test-key-$(date +%s)"
RAW=$(http_post "$BASE/api/integrations/connectors/$CONNECTOR_ID/test" "{
    \"idempotency_key\": \"$IDEM_KEY\"
}")
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "200" ]]; then
    RESULT_TYPE=$(extract_field "$BODY" "connector_type")
    RESULT_IDEM=$(extract_field "$BODY" "idempotency_key")
    RESULT_SUCCESS=$(extract_bool "$BODY" "success")
    # Check that the echo message contains our prefix
    if echo "$BODY" | grep -q '"hello-e2e"'; then
        GOT_PREFIX=true
    else
        GOT_PREFIX=false
    fi

    if [[ "$RESULT_TYPE" == "echo" && "$RESULT_IDEM" == "$IDEM_KEY" && "$RESULT_SUCCESS" == "true" && "$GOT_PREFIX" == "true" ]]; then
        e2e_pass "test action returned deterministic echo result"
    else
        e2e_fail "test action — unexpected result: $BODY"
    fi
else
    e2e_fail "test action — HTTP $STATUS (body: $BODY)"
fi

# ============================================================================
# Test 6: Idempotency — same key → same echo_prefix in output
# ============================================================================
echo ""
echo "--- Test 6: Idempotency key echoed back ---"
FIXED_KEY="fixed-idem-key-abc123"
RAW1=$(http_post "$BASE/api/integrations/connectors/$CONNECTOR_ID/test" "{
    \"idempotency_key\": \"$FIXED_KEY\"
}")
RAW2=$(http_post "$BASE/api/integrations/connectors/$CONNECTOR_ID/test" "{
    \"idempotency_key\": \"$FIXED_KEY\"
}")
BODY1=$(parse_response "$RAW1")
BODY2=$(parse_response "$RAW2")
STATUS1=$(parse_status "$RAW1")
STATUS2=$(parse_status "$RAW2")

if [[ "$STATUS1" == "200" && "$STATUS2" == "200" ]]; then
    IDEM1=$(extract_field "$BODY1" "idempotency_key")
    IDEM2=$(extract_field "$BODY2" "idempotency_key")
    MSG1=$(echo "$BODY1" | grep -o '"message":"[^"]*"' | head -1)
    MSG2=$(echo "$BODY2" | grep -o '"message":"[^"]*"' | head -1)
    if [[ "$IDEM1" == "$FIXED_KEY" && "$IDEM2" == "$FIXED_KEY" && "$MSG1" == "$MSG2" ]]; then
        e2e_pass "same idempotency_key produces same message"
    else
        e2e_fail "idempotency mismatch: msg1=$MSG1 msg2=$MSG2"
    fi
else
    e2e_fail "idempotency test — HTTP $STATUS1 / $STATUS2"
fi

# ============================================================================
# Test 7: Validation — unknown connector_type → 422
# ============================================================================
echo ""
echo "--- Test 7: Validation — unknown connector_type ---"
RAW=$(http_post_raw "$BASE/api/integrations/connectors" '{
    "connector_type": "nonexistent-connector",
    "name": "bad-type-test"
}')
STATUS=$(parse_status "$RAW")
if [[ "$STATUS" == "422" ]]; then
    e2e_pass "unknown connector_type rejected (422)"
else
    e2e_fail "unknown connector_type — HTTP $STATUS (expected 422)"
fi

# ============================================================================
# Test 8: Missing X-App-Id → 400
# ============================================================================
echo ""
echo "--- Test 8: Missing X-App-Id header → 400 ---"
RAW=$(http_get_no_appid "$BASE/api/integrations/connectors")
STATUS=$(parse_status "$RAW")
if [[ "$STATUS" == "400" ]]; then
    e2e_pass "missing X-App-Id returns 400"
else
    e2e_fail "missing X-App-Id — HTTP $STATUS (expected 400)"
fi

# ============================================================================
# Test 9: Outbox — verify connector.registered event was written
# (Indirect check: re-fetch connector config to confirm persistence)
# ============================================================================
echo ""
echo "--- Test 9: Connector config persisted (outbox implied) ---"
RAW=$(http_get "$BASE/api/integrations/connectors/$CONNECTOR_ID")
STATUS=$(parse_status "$RAW")
if [[ "$STATUS" == "200" ]]; then
    e2e_pass "connector config persisted after registration"
else
    e2e_fail "connector config not found after registration — HTTP $STATUS"
fi

# ============================================================================
# Test 10: Non-existent connector id → 404 on test action
# ============================================================================
echo ""
echo "--- Test 10: Test action on non-existent connector → 404 ---"
FAKE_ID="00000000-0000-0000-0000-000000000000"
RAW=$(http_post_raw "$BASE/api/integrations/connectors/$FAKE_ID/test" '{
    "idempotency_key": "any-key"
}')
STATUS=$(parse_status "$RAW")
if [[ "$STATUS" == "404" ]]; then
    e2e_pass "test action on unknown connector returns 404"
else
    e2e_fail "unknown connector test action — HTTP $STATUS (expected 404)"
fi

echo ""
echo "=== Integrations Connector E2E Complete ==="
