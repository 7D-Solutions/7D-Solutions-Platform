# TAGS: phase42-e2e consolidation consolidation-flow
# E2E test: consolidation module config CRUD flow via HTTP API
#
# Tests:
#   1. Service readiness
#   2. Create group + entity via psql (docker exec), verify via GET API
#   3. List groups returns created group
#   4. Get group by ID returns correct data
#   5. List entities for group returns created entity
#   6. Validate group returns expected shape
#   7. Negative: GET non-existent group → 404
#   8. Negative: POST without auth → 401
#   9. Negative: GET without X-App-Id → 400
#  10. Cleanup test data

CONSOLIDATION_PORT=$(resolve_port consolidation)
CSL_CONTAINER="7d-consolidation-postgres"
CSL_DB_USER="consolidation_user"
CSL_DB_NAME="consolidation_db"
BASE_URL="http://localhost:${CONSOLIDATION_PORT}"

echo "[consolidation-flow] port $CONSOLIDATION_PORT"

# Helper: run SQL against consolidation DB via docker exec
csl_psql() {
    docker exec -i "$CSL_CONTAINER" psql -U "$CSL_DB_USER" -d "$CSL_DB_NAME" -q -t "$@" 2>/dev/null
}

# Wait for service readiness
if ! wait_for_ready "consolidation" "$CONSOLIDATION_PORT" "${E2E_TIMEOUT:-30}"; then
    e2e_skip "consolidation-flow: service not ready, skipping flow tests"
    return 0 2>/dev/null || true
fi

# Verify DB container is reachable
if ! docker exec "$CSL_CONTAINER" pg_isready -U "$CSL_DB_USER" -d "$CSL_DB_NAME" >/dev/null 2>&1; then
    e2e_skip "consolidation-flow: DB container not reachable, skipping"
    return 0 2>/dev/null || true
fi

# Generate namespaced test identifiers (idempotent: UUID suffix)
TEST_TENANT="$(bootstrap_test_tenant csl-flow)"
TEST_GROUP_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
TEST_ENTITY_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
FAKE_GROUP_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"

echo "[consolidation-flow] tenant=$TEST_TENANT group=$TEST_GROUP_ID"

# ── Cleanup function (idempotent) ──────────────────────────────────
consolidation_flow_cleanup() {
    echo "DELETE FROM csl_group_entities WHERE group_id = '$TEST_GROUP_ID';
          DELETE FROM csl_groups WHERE id = '$TEST_GROUP_ID';" \
        | csl_psql || true
}

# Clean up any leftover data from a previous run with same IDs (unlikely but safe)
consolidation_flow_cleanup

# ── Step 1: Insert test data via psql ──────────────────────────────
if ! echo "
    INSERT INTO csl_groups (id, tenant_id, name, reporting_currency, fiscal_year_end_month)
    VALUES ('$TEST_GROUP_ID', '$TEST_TENANT', 'E2E Flow Test Group', 'USD', 12);
" | csl_psql; then
    e2e_fail "consolidation-flow: failed to insert test group via psql"
    return 0 2>/dev/null || true
fi

if ! echo "
    INSERT INTO csl_group_entities
        (id, group_id, entity_tenant_id, entity_name, functional_currency, ownership_pct_bp, consolidation_method)
    VALUES
        ('$TEST_ENTITY_ID', '$TEST_GROUP_ID', '${TEST_TENANT}-sub', 'E2E Sub Entity', 'EUR', 10000, 'full');
" | csl_psql; then
    e2e_fail "consolidation-flow: failed to insert test entity via psql"
    consolidation_flow_cleanup
    return 0 2>/dev/null || true
fi

e2e_pass "consolidation-flow: test data seeded via psql"

# ── Step 2: List groups via API ────────────────────────────────────
LIST_RESP=$(curl -sf -m 5 \
    -H "X-App-Id: $TEST_TENANT" \
    "$BASE_URL/api/consolidation/groups" 2>/dev/null) || LIST_RESP=""

if [[ -n "$LIST_RESP" ]] && echo "$LIST_RESP" | grep -q "$TEST_GROUP_ID"; then
    e2e_pass "consolidation-flow: list groups contains created group"
else
    e2e_fail "consolidation-flow: list groups did not contain group $TEST_GROUP_ID"
fi

# ── Step 3: Get group by ID ───────────────────────────────────────
GET_RAW=$(curl -s -m 5 -w '\n%{http_code}' \
    -H "X-App-Id: $TEST_TENANT" \
    "$BASE_URL/api/consolidation/groups/$TEST_GROUP_ID" 2>/dev/null)
GET_CODE=$(echo "$GET_RAW" | tail -1)
GET_BODY=$(echo "$GET_RAW" | sed '$d')

if [[ "$GET_CODE" == "200" ]] && echo "$GET_BODY" | grep -q '"reporting_currency":"USD"'; then
    e2e_pass "consolidation-flow: get group returns 200 with correct currency"
else
    e2e_fail "consolidation-flow: get group returned HTTP $GET_CODE (expected 200 with USD)"
fi

# ── Step 4: List entities for group ────────────────────────────────
ENT_RESP=$(curl -sf -m 5 \
    -H "X-App-Id: $TEST_TENANT" \
    "$BASE_URL/api/consolidation/groups/$TEST_GROUP_ID/entities" 2>/dev/null) || ENT_RESP=""

if [[ -n "$ENT_RESP" ]] && echo "$ENT_RESP" | grep -q "$TEST_ENTITY_ID"; then
    e2e_pass "consolidation-flow: list entities contains created entity"
else
    e2e_fail "consolidation-flow: list entities did not contain entity $TEST_ENTITY_ID"
fi

# ── Step 5: Validate group shape ──────────────────────────────────
VAL_RAW=$(curl -s -m 5 -w '\n%{http_code}' \
    -H "X-App-Id: $TEST_TENANT" \
    "$BASE_URL/api/consolidation/groups/$TEST_GROUP_ID/validate" 2>/dev/null)
VAL_CODE=$(echo "$VAL_RAW" | tail -1)
VAL_BODY=$(echo "$VAL_RAW" | sed '$d')

if [[ "$VAL_CODE" == "200" ]] && echo "$VAL_BODY" | grep -q '"is_complete"'; then
    e2e_pass "consolidation-flow: validate group returns shape with is_complete"
else
    e2e_fail "consolidation-flow: validate group HTTP $VAL_CODE, missing is_complete"
fi

# ── Negative: GET non-existent group → 404 ────────────────────────
NOT_FOUND_CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    -H "X-App-Id: $TEST_TENANT" \
    "$BASE_URL/api/consolidation/groups/$FAKE_GROUP_ID" 2>/dev/null) || NOT_FOUND_CODE="000"

if [[ "$NOT_FOUND_CODE" == "404" ]]; then
    e2e_pass "consolidation-flow: get non-existent group returns 404"
else
    e2e_fail "consolidation-flow: get non-existent group returned $NOT_FOUND_CODE (expected 404)"
fi

# ── Negative: POST group without auth → 401 ──────────────────────
UNAUTH_CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    -X POST \
    -H "Content-Type: application/json" \
    -H "X-App-Id: $TEST_TENANT" \
    -d '{"name":"unauth group","reporting_currency":"USD"}' \
    "$BASE_URL/api/consolidation/groups" 2>/dev/null) || UNAUTH_CODE="000"

if [[ "$UNAUTH_CODE" == "401" ]]; then
    e2e_pass "consolidation-flow: POST without auth returns 401"
else
    e2e_fail "consolidation-flow: POST without auth returned $UNAUTH_CODE (expected 401)"
fi

# ── Negative: GET groups without X-App-Id → 400 ──────────────────
NO_APPID_CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    "$BASE_URL/api/consolidation/groups" 2>/dev/null) || NO_APPID_CODE="000"

if [[ "$NO_APPID_CODE" == "400" ]]; then
    e2e_pass "consolidation-flow: GET without X-App-Id returns 400"
else
    e2e_fail "consolidation-flow: GET without X-App-Id returned $NO_APPID_CODE (expected 400)"
fi

# ── Cleanup ────────────────────────────────────────────────────────
consolidation_flow_cleanup
echo "[consolidation-flow] cleanup complete"
