#!/usr/bin/env bash
# TAGS: party party-crud phase42
# E2E: Party CRUD — org, contacts, addresses full lifecycle.
#
# Tests:
#   1. Create company org
#   2. Add contact to org
#   3. Add billing address to org
#   4. Fetch org — verify contacts + addresses included
#   5. Update contact
#   6. Update address
#   7. Delete contact
#   8. Delete address
#   9. Deactivate org — verify soft-delete
#  10. Validation: reject empty display_name (422)
#  11. Validation: reject invalid email on contact (422)
#  12. Validation: reject empty line1 on address (422)
#  13. Validation: reject invalid address_type (422)

# Helpers are sourced by the runner (scripts/e2e_run.sh).

PARTY_PORT=$(resolve_port party)

echo "=== Party CRUD E2E ==="
echo "[party-crud] port $PARTY_PORT"

# Wait for service readiness
if ! wait_for_ready "party" "$PARTY_PORT" "${E2E_TIMEOUT:-30}"; then
    e2e_skip "party service not ready — skipping CRUD tests"
    return 0 2>/dev/null || true
fi

BASE="http://localhost:${PARTY_PORT}"
APP_ID="e2e-party-crud-$(date +%s)"
HEADERS=(-H "Content-Type: application/json" -H "X-App-Id: $APP_ID" -H "X-Correlation-Id: e2e-corr-1")

# ============================================================================
# Helper: HTTP request with status code capture
# ============================================================================
http_post() {
    local url="$1" body="$2"
    curl -sf -w '\n%{http_code}' -X POST "$url" "${HEADERS[@]}" -d "$body" 2>/dev/null
}

http_get() {
    local url="$1"
    curl -sf -w '\n%{http_code}' "$url" "${HEADERS[@]}" 2>/dev/null
}

http_put() {
    local url="$1" body="$2"
    curl -sf -w '\n%{http_code}' -X PUT "$url" "${HEADERS[@]}" -d "$body" 2>/dev/null
}

http_delete() {
    local url="$1"
    curl -s -o /dev/null -w '%{http_code}' -X DELETE "$url" "${HEADERS[@]}" 2>/dev/null
}

# Raw request for validation tests (don't use -f so we get error bodies)
http_post_raw() {
    local url="$1" body="$2"
    curl -s -w '\n%{http_code}' -X POST "$url" "${HEADERS[@]}" -d "$body" 2>/dev/null
}

parse_response() {
    local raw="$1"
    local body http_code
    http_code=$(echo "$raw" | tail -1)
    body=$(echo "$raw" | sed '$d')
    echo "$body"
    return 0
}

parse_status() {
    local raw="$1"
    echo "$raw" | tail -1
}

extract_json_field() {
    local json="$1" field="$2"
    echo "$json" | grep -o "\"${field}\":\"[^\"]*\"" | head -1 | cut -d'"' -f4
}

extract_json_bool() {
    local json="$1" field="$2"
    echo "$json" | grep -o "\"${field}\":[a-z]*" | head -1 | cut -d: -f2
}

extract_json_array_len() {
    local json="$1" field="$2"
    # Count array elements by counting opening braces after the field
    local arr
    arr=$(echo "$json" | grep -o "\"${field}\":\[[^]]*\]" | head -1)
    if [[ "$arr" == *"[]"* ]] || [[ -z "$arr" ]]; then
        echo "0"
    else
        echo "$arr" | grep -o '{' | wc -l | tr -d ' '
    fi
}

# ============================================================================
# Test 1: Create company org
# ============================================================================
echo ""
echo "--- Test 1: Create company org ---"
RAW=$(http_post "$BASE/api/party/companies" '{
    "display_name": "Acme E2E Corp",
    "legal_name": "Acme E2E Corp LLC",
    "email": "info@acme-e2e.com",
    "country": "US"
}')
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "201" ]]; then
    ORG_ID=$(extract_json_field "$BODY" "id")
    ORG_NAME=$(extract_json_field "$BODY" "display_name")
    if [[ -n "$ORG_ID" && "$ORG_NAME" == "Acme E2E Corp" ]]; then
        e2e_pass "create company org (id=$ORG_ID)"
    else
        e2e_fail "create company org — unexpected body"
    fi
else
    e2e_fail "create company org — HTTP $STATUS"
    # Cannot continue without org
    return 0 2>/dev/null || true
fi

# ============================================================================
# Test 2: Add contact to org
# ============================================================================
echo ""
echo "--- Test 2: Add contact to org ---"
RAW=$(http_post "$BASE/api/party/parties/$ORG_ID/contacts" '{
    "first_name": "Jane",
    "last_name": "Doe",
    "email": "jane.doe@acme-e2e.com",
    "phone": "+1-555-0101",
    "role": "billing",
    "is_primary": true
}')
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "201" ]]; then
    CONTACT_ID=$(extract_json_field "$BODY" "id")
    CONTACT_FIRST=$(extract_json_field "$BODY" "first_name")
    CONTACT_PRIMARY=$(extract_json_bool "$BODY" "is_primary")
    if [[ "$CONTACT_FIRST" == "Jane" && "$CONTACT_PRIMARY" == "true" ]]; then
        e2e_pass "create contact (id=$CONTACT_ID)"
    else
        e2e_fail "create contact — unexpected body: $BODY"
    fi
else
    e2e_fail "create contact — HTTP $STATUS"
fi

# ============================================================================
# Test 3: Add billing address to org
# ============================================================================
echo ""
echo "--- Test 3: Add billing address ---"
RAW=$(http_post "$BASE/api/party/parties/$ORG_ID/addresses" '{
    "address_type": "billing",
    "label": "HQ",
    "line1": "123 Main St",
    "line2": "Suite 100",
    "city": "Portland",
    "state": "OR",
    "postal_code": "97201",
    "country": "US",
    "is_primary": true
}')
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "201" ]]; then
    ADDRESS_ID=$(extract_json_field "$BODY" "id")
    ADDR_TYPE=$(extract_json_field "$BODY" "address_type")
    ADDR_CITY=$(extract_json_field "$BODY" "city")
    if [[ "$ADDR_TYPE" == "billing" && "$ADDR_CITY" == "Portland" ]]; then
        e2e_pass "create billing address (id=$ADDRESS_ID)"
    else
        e2e_fail "create address — unexpected body: $BODY"
    fi
else
    e2e_fail "create address — HTTP $STATUS"
fi

# ============================================================================
# Test 4: Fetch org — verify contacts + addresses included
# ============================================================================
echo ""
echo "--- Test 4: Fetch org with linked entities ---"
RAW=$(http_get "$BASE/api/party/parties/$ORG_ID")
STATUS=$(parse_status "$RAW")
BODY=$(parse_response "$RAW")

if [[ "$STATUS" == "200" ]]; then
    CONTACT_COUNT=$(extract_json_array_len "$BODY" "contacts")
    ADDRESS_COUNT=$(extract_json_array_len "$BODY" "addresses")
    FETCHED_NAME=$(extract_json_field "$BODY" "display_name")
    if [[ "$FETCHED_NAME" == "Acme E2E Corp" && "$CONTACT_COUNT" -ge 1 && "$ADDRESS_COUNT" -ge 1 ]]; then
        e2e_pass "fetch org includes contacts ($CONTACT_COUNT) + addresses ($ADDRESS_COUNT)"
    else
        e2e_fail "fetch org — contacts=$CONTACT_COUNT addresses=$ADDRESS_COUNT name=$FETCHED_NAME"
    fi
else
    e2e_fail "fetch org — HTTP $STATUS"
fi

# ============================================================================
# Test 5: Update contact
# ============================================================================
echo ""
echo "--- Test 5: Update contact ---"
if [[ -n "${CONTACT_ID:-}" ]]; then
    RAW=$(http_put "$BASE/api/party/contacts/$CONTACT_ID" '{
        "first_name": "Janet",
        "role": "primary"
    }')
    STATUS=$(parse_status "$RAW")
    BODY=$(parse_response "$RAW")

    if [[ "$STATUS" == "200" ]]; then
        UPDATED_FIRST=$(extract_json_field "$BODY" "first_name")
        UPDATED_ROLE=$(extract_json_field "$BODY" "role")
        if [[ "$UPDATED_FIRST" == "Janet" && "$UPDATED_ROLE" == "primary" ]]; then
            e2e_pass "update contact (first_name=Janet, role=primary)"
        else
            e2e_fail "update contact — unexpected: first=$UPDATED_FIRST role=$UPDATED_ROLE"
        fi
    else
        e2e_fail "update contact — HTTP $STATUS"
    fi
else
    e2e_skip "update contact — no contact_id"
fi

# ============================================================================
# Test 6: Update address
# ============================================================================
echo ""
echo "--- Test 6: Update address ---"
if [[ -n "${ADDRESS_ID:-}" ]]; then
    RAW=$(http_put "$BASE/api/party/addresses/$ADDRESS_ID" '{
        "city": "Seattle",
        "state": "WA",
        "postal_code": "98101"
    }')
    STATUS=$(parse_status "$RAW")
    BODY=$(parse_response "$RAW")

    if [[ "$STATUS" == "200" ]]; then
        UPDATED_CITY=$(extract_json_field "$BODY" "city")
        if [[ "$UPDATED_CITY" == "Seattle" ]]; then
            e2e_pass "update address (city=Seattle)"
        else
            e2e_fail "update address — city=$UPDATED_CITY"
        fi
    else
        e2e_fail "update address — HTTP $STATUS"
    fi
else
    e2e_skip "update address — no address_id"
fi

# ============================================================================
# Test 7: Delete contact
# ============================================================================
echo ""
echo "--- Test 7: Delete contact ---"
if [[ -n "${CONTACT_ID:-}" ]]; then
    STATUS=$(http_delete "$BASE/api/party/contacts/$CONTACT_ID")
    if [[ "$STATUS" == "204" ]]; then
        e2e_pass "delete contact (204)"
    else
        e2e_fail "delete contact — HTTP $STATUS"
    fi

    # Verify gone
    VERIFY_STATUS=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/party/contacts/$CONTACT_ID" "${HEADERS[@]}" 2>/dev/null)
    if [[ "$VERIFY_STATUS" == "404" ]]; then
        e2e_pass "deleted contact returns 404"
    else
        e2e_fail "deleted contact returns $VERIFY_STATUS (expected 404)"
    fi
else
    e2e_skip "delete contact — no contact_id"
fi

# ============================================================================
# Test 8: Delete address
# ============================================================================
echo ""
echo "--- Test 8: Delete address ---"
if [[ -n "${ADDRESS_ID:-}" ]]; then
    STATUS=$(http_delete "$BASE/api/party/addresses/$ADDRESS_ID")
    if [[ "$STATUS" == "204" ]]; then
        e2e_pass "delete address (204)"
    else
        e2e_fail "delete address — HTTP $STATUS"
    fi

    # Verify gone
    VERIFY_STATUS=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/party/addresses/$ADDRESS_ID" "${HEADERS[@]}" 2>/dev/null)
    if [[ "$VERIFY_STATUS" == "404" ]]; then
        e2e_pass "deleted address returns 404"
    else
        e2e_fail "deleted address returns $VERIFY_STATUS (expected 404)"
    fi
else
    e2e_skip "delete address — no address_id"
fi

# ============================================================================
# Test 9: Deactivate org (soft-delete)
# ============================================================================
echo ""
echo "--- Test 9: Deactivate org ---"
RAW=$(http_post "$BASE/api/party/parties/$ORG_ID/deactivate" '{}')
STATUS=$(parse_status "$RAW")

if [[ "$STATUS" == "204" ]]; then
    e2e_pass "deactivate org (204)"

    # Should not appear in active list
    RAW=$(http_get "$BASE/api/party/parties")
    BODY=$(parse_response "$RAW")
    if echo "$BODY" | grep -q "$ORG_ID"; then
        e2e_fail "deactivated org still in active list"
    else
        e2e_pass "deactivated org excluded from active list"
    fi

    # Should appear with include_inactive
    RAW=$(http_get "$BASE/api/party/parties?include_inactive=true")
    BODY=$(parse_response "$RAW")
    if echo "$BODY" | grep -q "$ORG_ID"; then
        e2e_pass "deactivated org visible with include_inactive=true"
    else
        e2e_fail "deactivated org not found with include_inactive=true"
    fi
else
    e2e_fail "deactivate org — HTTP $STATUS"
fi

# ============================================================================
# Test 10: Validation — empty display_name (422)
# ============================================================================
echo ""
echo "--- Test 10: Validation — empty display_name ---"
RAW=$(http_post_raw "$BASE/api/party/companies" '{
    "display_name": "",
    "legal_name": "Empty Name Corp"
}')
STATUS=$(parse_status "$RAW")
if [[ "$STATUS" == "422" ]]; then
    e2e_pass "empty display_name rejected (422)"
else
    e2e_fail "empty display_name — HTTP $STATUS (expected 422)"
fi

# ============================================================================
# Test 11: Validation — invalid email on contact (422)
# ============================================================================
echo ""
echo "--- Test 11: Validation — invalid email on contact ---"
# First create a fresh org for validation tests
RAW=$(http_post "$BASE/api/party/companies" '{
    "display_name": "Validation Test Corp",
    "legal_name": "Validation Test Corp LLC"
}')
VAL_ORG_ID=$(extract_json_field "$(parse_response "$RAW")" "id")

if [[ -n "$VAL_ORG_ID" ]]; then
    RAW=$(http_post_raw "$BASE/api/party/parties/$VAL_ORG_ID/contacts" '{
        "first_name": "Bad",
        "last_name": "Email",
        "email": "not-an-email"
    }')
    STATUS=$(parse_status "$RAW")
    if [[ "$STATUS" == "422" ]]; then
        e2e_pass "invalid email on contact rejected (422)"
    else
        e2e_fail "invalid email on contact — HTTP $STATUS (expected 422)"
    fi
else
    e2e_skip "validation test — could not create test org"
fi

# ============================================================================
# Test 12: Validation — empty line1 on address (422)
# ============================================================================
echo ""
echo "--- Test 12: Validation — empty line1 on address ---"
if [[ -n "$VAL_ORG_ID" ]]; then
    RAW=$(http_post_raw "$BASE/api/party/parties/$VAL_ORG_ID/addresses" '{
        "line1": "",
        "city": "Portland"
    }')
    STATUS=$(parse_status "$RAW")
    if [[ "$STATUS" == "422" ]]; then
        e2e_pass "empty line1 on address rejected (422)"
    else
        e2e_fail "empty line1 on address — HTTP $STATUS (expected 422)"
    fi
else
    e2e_skip "validation test — no test org"
fi

# ============================================================================
# Test 13: Validation — invalid address_type (422)
# ============================================================================
echo ""
echo "--- Test 13: Validation — invalid address_type ---"
if [[ -n "$VAL_ORG_ID" ]]; then
    RAW=$(http_post_raw "$BASE/api/party/parties/$VAL_ORG_ID/addresses" '{
        "address_type": "nonsense",
        "line1": "123 Main",
        "city": "Portland"
    }')
    STATUS=$(parse_status "$RAW")
    if [[ "$STATUS" == "422" ]]; then
        e2e_pass "invalid address_type rejected (422)"
    else
        e2e_fail "invalid address_type — HTTP $STATUS (expected 422)"
    fi
else
    e2e_skip "validation test — no test org"
fi

# ============================================================================
# Cleanup: delete test data
# ============================================================================
echo ""
echo "--- Cleanup ---"
# Deactivate validation org if created
if [[ -n "${VAL_ORG_ID:-}" ]]; then
    http_post "$BASE/api/party/parties/$VAL_ORG_ID/deactivate" '{}' >/dev/null 2>&1 || true
fi

echo ""
echo "=== Party CRUD E2E Complete ==="
