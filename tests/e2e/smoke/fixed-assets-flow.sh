# TAGS: phase42-e2e fixed-assets fixed-assets-flow
# E2E test: fixed-assets module asset lifecycle flow via HTTP API
#
# Tests:
#   1. Service readiness
#   2. Create category + asset via psql (docker exec), verify via GET API
#   3. List categories returns created category
#   4. Get category by ID returns correct data
#   5. List assets returns created asset
#   6. Get asset by ID returns correct stable fields (cost, currency, status)
#   7. Seed depreciation run + schedule via psql, verify run via GET API
#   8. Verify depreciation run stable outputs (periods_posted, total, status)
#   9. Negative: GET non-existent asset -> 404
#  10. Negative: POST without auth -> 401
#  11. Negative: GET non-existent category -> 404
#  12. Idempotent: repeated GET returns identical stable data
#  13. Cleanup test data

FA_PORT=$(resolve_port fixed-assets)
FA_CONTAINER="7d-fixed-assets-postgres"
FA_DB_USER="fixed_assets_user"
FA_DB_NAME="fixed_assets_db"
BASE_URL="http://localhost:${FA_PORT}"

echo "[fixed-assets-flow] port $FA_PORT"

# Helper: run SQL against fixed-assets DB via docker exec
fa_psql() {
    docker exec -i "$FA_CONTAINER" psql -U "$FA_DB_USER" -d "$FA_DB_NAME" -q -t "$@" 2>/dev/null
}

# Wait for service readiness
if ! wait_for_ready "fixed-assets" "$FA_PORT" "${E2E_TIMEOUT:-30}"; then
    e2e_skip "fixed-assets-flow: service not ready, skipping flow tests"
    return 0 2>/dev/null || true
fi

# Verify DB container is reachable
if ! docker exec "$FA_CONTAINER" pg_isready -U "$FA_DB_USER" -d "$FA_DB_NAME" >/dev/null 2>&1; then
    e2e_skip "fixed-assets-flow: DB container not reachable, skipping"
    return 0 2>/dev/null || true
fi

# Generate namespaced test identifiers (unique per run)
TEST_TENANT="$(bootstrap_test_tenant fa-flow)"
TEST_CAT_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
TEST_ASSET_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
TEST_RUN_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
FAKE_ASSET_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
FAKE_CAT_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
TEST_CAT_CODE="E2E-$(echo "$TEST_CAT_ID" | cut -c1-8)"
TEST_ASSET_TAG="FA-$(echo "$TEST_ASSET_ID" | cut -c1-8)"

echo "[fixed-assets-flow] tenant=$TEST_TENANT cat=$TEST_CAT_ID asset=$TEST_ASSET_ID"

# ── Cleanup function (idempotent) ──────────────────────────────────
fa_flow_cleanup() {
    echo "
        DELETE FROM fa_depreciation_schedules WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_depreciation_runs WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_disposals WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_events_outbox WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_ap_capitalizations WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_assets WHERE tenant_id = '$TEST_TENANT';
        DELETE FROM fa_categories WHERE tenant_id = '$TEST_TENANT';
    " | fa_psql || true
}

# Clean up any leftover data from a previous run
fa_flow_cleanup

# ── Step 1: Seed category via psql ─────────────────────────────────
if ! echo "
    INSERT INTO fa_categories
        (id, tenant_id, code, name, description,
         default_method, default_useful_life_months, default_salvage_pct_bp,
         asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
         gain_loss_account_ref, is_active, created_at, updated_at)
    VALUES
        ('$TEST_CAT_ID', '$TEST_TENANT', '$TEST_CAT_CODE', 'E2E Furniture',
         'Test category for E2E', 'straight_line', 60, 500,
         '1500', '6100', '1510', '7000', TRUE, NOW(), NOW());
" | fa_psql; then
    e2e_fail "fixed-assets-flow: failed to insert test category"
    return 0 2>/dev/null || true
fi

# ── Step 2: Seed asset via psql ────────────────────────────────────
if ! echo "
    INSERT INTO fa_assets
        (id, tenant_id, category_id, asset_tag, name, description,
         status, acquisition_date, in_service_date,
         acquisition_cost_minor, currency,
         depreciation_method, useful_life_months, salvage_value_minor,
         accum_depreciation_minor, net_book_value_minor,
         location, department,
         created_at, updated_at)
    VALUES
        ('$TEST_ASSET_ID', '$TEST_TENANT', '$TEST_CAT_ID',
         '$TEST_ASSET_TAG', 'E2E Office Desk', 'Desk for flow test',
         'active', '2026-01-01', '2026-01-01',
         120000, 'USD',
         'straight_line', 60, 0,
         0, 120000,
         'Floor 3', 'Engineering',
         NOW(), NOW());
" | fa_psql; then
    e2e_fail "fixed-assets-flow: failed to insert test asset"
    fa_flow_cleanup
    return 0 2>/dev/null || true
fi

e2e_pass "fixed-assets-flow: test data seeded via psql"

# ── Step 3: List categories via API ─────────────────────────────────
LIST_CAT_RESP=$(curl -sf -m 5 \
    "$BASE_URL/api/fixed-assets/categories/$TEST_TENANT" 2>/dev/null) || LIST_CAT_RESP=""

if [[ -n "$LIST_CAT_RESP" ]] && echo "$LIST_CAT_RESP" | grep -q "$TEST_CAT_ID"; then
    e2e_pass "fixed-assets-flow: list categories contains created category"
else
    e2e_fail "fixed-assets-flow: list categories missing category $TEST_CAT_ID"
fi

# ── Step 4: Get category by ID ──────────────────────────────────────
CAT_RAW=$(curl -s -m 5 -w '\n%{http_code}' \
    "$BASE_URL/api/fixed-assets/categories/$TEST_TENANT/$TEST_CAT_ID" 2>/dev/null)
CAT_CODE=$(echo "$CAT_RAW" | tail -1)
CAT_BODY=$(echo "$CAT_RAW" | sed '$d')

if [[ "$CAT_CODE" == "200" ]] \
    && echo "$CAT_BODY" | grep -q "\"code\":\"$TEST_CAT_CODE\"" \
    && echo "$CAT_BODY" | grep -q '"asset_account_ref":"1500"' \
    && echo "$CAT_BODY" | grep -q '"default_useful_life_months":60'; then
    e2e_pass "fixed-assets-flow: get category returns correct stable fields"
else
    e2e_fail "fixed-assets-flow: get category HTTP $CAT_CODE, unexpected body"
fi

# ── Step 5: List assets via API ──────────────────────────────────────
LIST_ASSET_RESP=$(curl -sf -m 5 \
    "$BASE_URL/api/fixed-assets/assets/$TEST_TENANT" 2>/dev/null) || LIST_ASSET_RESP=""

if [[ -n "$LIST_ASSET_RESP" ]] && echo "$LIST_ASSET_RESP" | grep -q "$TEST_ASSET_ID"; then
    e2e_pass "fixed-assets-flow: list assets contains created asset"
else
    e2e_fail "fixed-assets-flow: list assets missing asset $TEST_ASSET_ID"
fi

# ── Step 6: Get asset by ID — validate stable financial fields ──────
ASSET_RAW=$(curl -s -m 5 -w '\n%{http_code}' \
    "$BASE_URL/api/fixed-assets/assets/$TEST_TENANT/$TEST_ASSET_ID" 2>/dev/null)
ASSET_CODE=$(echo "$ASSET_RAW" | tail -1)
ASSET_BODY=$(echo "$ASSET_RAW" | sed '$d')

if [[ "$ASSET_CODE" == "200" ]] \
    && echo "$ASSET_BODY" | grep -q "\"asset_tag\":\"$TEST_ASSET_TAG\"" \
    && echo "$ASSET_BODY" | grep -q '"acquisition_cost_minor":120000' \
    && echo "$ASSET_BODY" | grep -q '"currency":"USD"' \
    && echo "$ASSET_BODY" | grep -q '"status":"active"' \
    && echo "$ASSET_BODY" | grep -q '"net_book_value_minor":120000' \
    && echo "$ASSET_BODY" | grep -q '"useful_life_months":60'; then
    e2e_pass "fixed-assets-flow: get asset returns correct stable financial fields"
else
    e2e_fail "fixed-assets-flow: get asset HTTP $ASSET_CODE, unexpected body"
fi

# ── Step 7: Seed depreciation run + schedule via psql ────────────────
IDEMPOTENCY_KEY="$(uuidgen | tr '[:upper:]' '[:lower:]')"

# Insert a completed depreciation run (6 months of depreciation)
if ! echo "
    INSERT INTO fa_depreciation_runs
        (id, tenant_id, as_of_date, status,
         assets_processed, periods_posted, total_depreciation_minor,
         currency, idempotency_key,
         started_at, completed_at, created_at, updated_at)
    VALUES
        ('$TEST_RUN_ID', '$TEST_TENANT', '2026-06-30', 'completed',
         1, 6, 12000, 'USD', '$IDEMPOTENCY_KEY',
         NOW(), NOW(), NOW(), NOW());
" | fa_psql; then
    e2e_fail "fixed-assets-flow: failed to insert depreciation run"
    fa_flow_cleanup
    return 0 2>/dev/null || true
fi

e2e_pass "fixed-assets-flow: depreciation run seeded"

# ── Step 8: List depreciation runs via API ──────────────────────────
LIST_RUN_RESP=$(curl -sf -m 5 \
    "$BASE_URL/api/fixed-assets/depreciation/runs/$TEST_TENANT" 2>/dev/null) || LIST_RUN_RESP=""

if [[ -n "$LIST_RUN_RESP" ]] && echo "$LIST_RUN_RESP" | grep -q "$TEST_RUN_ID"; then
    e2e_pass "fixed-assets-flow: list runs contains created run"
else
    e2e_fail "fixed-assets-flow: list runs missing run $TEST_RUN_ID"
fi

# ── Step 9: Get run by ID — validate stable outputs ─────────────────
RUN_RAW=$(curl -s -m 5 -w '\n%{http_code}' \
    "$BASE_URL/api/fixed-assets/depreciation/runs/$TEST_TENANT/$TEST_RUN_ID" 2>/dev/null)
RUN_CODE=$(echo "$RUN_RAW" | tail -1)
RUN_BODY=$(echo "$RUN_RAW" | sed '$d')

if [[ "$RUN_CODE" == "200" ]] \
    && echo "$RUN_BODY" | grep -q '"status":"completed"' \
    && echo "$RUN_BODY" | grep -q '"periods_posted":6' \
    && echo "$RUN_BODY" | grep -q '"total_depreciation_minor":12000' \
    && echo "$RUN_BODY" | grep -q '"assets_processed":1' \
    && echo "$RUN_BODY" | grep -q '"currency":"USD"'; then
    e2e_pass "fixed-assets-flow: get run returns correct stable depreciation outputs"
else
    e2e_fail "fixed-assets-flow: get run HTTP $RUN_CODE, unexpected body"
fi

# ── Negative: GET non-existent asset -> 404 ──────────────────────────
NOT_FOUND_ASSET=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    "$BASE_URL/api/fixed-assets/assets/$TEST_TENANT/$FAKE_ASSET_ID" 2>/dev/null) || NOT_FOUND_ASSET="000"

if [[ "$NOT_FOUND_ASSET" == "404" ]]; then
    e2e_pass "fixed-assets-flow: get non-existent asset returns 404"
else
    e2e_fail "fixed-assets-flow: get non-existent asset returned $NOT_FOUND_ASSET (expected 404)"
fi

# ── Negative: GET non-existent category -> 404 ──────────────────────
NOT_FOUND_CAT=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    "$BASE_URL/api/fixed-assets/categories/$TEST_TENANT/$FAKE_CAT_ID" 2>/dev/null) || NOT_FOUND_CAT="000"

if [[ "$NOT_FOUND_CAT" == "404" ]]; then
    e2e_pass "fixed-assets-flow: get non-existent category returns 404"
else
    e2e_fail "fixed-assets-flow: get non-existent category returned $NOT_FOUND_CAT (expected 404)"
fi

# ── Negative: POST category without auth -> 401 ─────────────────────
UNAUTH_CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    -X POST \
    -H "Content-Type: application/json" \
    -d '{"tenant_id":"'$TEST_TENANT'","code":"NOAUTH","name":"No Auth Cat","asset_account_ref":"1500","depreciation_expense_ref":"6100","accum_depreciation_ref":"1510"}' \
    "$BASE_URL/api/fixed-assets/categories" 2>/dev/null) || UNAUTH_CODE="000"

if [[ "$UNAUTH_CODE" == "401" ]]; then
    e2e_pass "fixed-assets-flow: POST category without auth returns 401"
else
    e2e_fail "fixed-assets-flow: POST category without auth returned $UNAUTH_CODE (expected 401)"
fi

# ── Negative: POST asset without auth -> 401 ────────────────────────
UNAUTH_ASSET_CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
    -X POST \
    -H "Content-Type: application/json" \
    -d '{"tenant_id":"'$TEST_TENANT'","category_id":"'$TEST_CAT_ID'","asset_tag":"NOAUTH-01","name":"No Auth Asset","acquisition_date":"2026-01-01","acquisition_cost_minor":1000}' \
    "$BASE_URL/api/fixed-assets/assets" 2>/dev/null) || UNAUTH_ASSET_CODE="000"

if [[ "$UNAUTH_ASSET_CODE" == "401" ]]; then
    e2e_pass "fixed-assets-flow: POST asset without auth returns 401"
else
    e2e_fail "fixed-assets-flow: POST asset without auth returned $UNAUTH_ASSET_CODE (expected 401)"
fi

# ── Idempotent: repeated GET returns identical stable data ───────────
ASSET_RAW2=$(curl -s -m 5 -w '\n%{http_code}' \
    "$BASE_URL/api/fixed-assets/assets/$TEST_TENANT/$TEST_ASSET_ID" 2>/dev/null)
ASSET_CODE2=$(echo "$ASSET_RAW2" | tail -1)
ASSET_BODY2=$(echo "$ASSET_RAW2" | sed '$d')

if [[ "$ASSET_CODE2" == "200" ]] \
    && echo "$ASSET_BODY2" | grep -q '"acquisition_cost_minor":120000' \
    && echo "$ASSET_BODY2" | grep -q '"net_book_value_minor":120000' \
    && echo "$ASSET_BODY2" | grep -q '"currency":"USD"'; then
    e2e_pass "fixed-assets-flow: idempotent GET returns identical stable fields"
else
    e2e_fail "fixed-assets-flow: idempotent GET returned different data"
fi

# ── Cleanup ──────────────────────────────────────────────────────────
fa_flow_cleanup
echo "[fixed-assets-flow] cleanup complete"
