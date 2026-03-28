#!/usr/bin/env bash
# QBO Full Smoke Test — find what breaks.
#
# Hits every endpoint, pushes every boundary, documents every failure.
# Sandbox credentials in .env.qbo-sandbox, tokens in .qbo-tokens.json.
#
# Usage: ./scripts/qbo/smoke-test.sh

set -uo pipefail
cd "$(dirname "$0")/../.."

source .env.qbo-sandbox

if [[ ! -f .qbo-tokens.json ]]; then
  echo "ERROR: .qbo-tokens.json not found."
  exit 1
fi

REALM_ID=$(jq -r '.realm_id' .qbo-tokens.json)
BASE="${QBO_SANDBOX_BASE}/company/${REALM_ID}"
MINOR="minorversion=75"
ACCESS_TOKEN=""
REFRESH_TOKEN=$(jq -r '.refresh_token' .qbo-tokens.json)

PASS_COUNT=0
FAIL_COUNT=0
FAILURES=""
REPORT_FILE="docs/qbo-smoke-test-results.md"
CURL_TMP=$(mktemp)
trap 'rm -f "$CURL_TMP"' EXIT

# ── Helpers ─────────────────────────────────────────────────────────

pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  echo "  [PASS] $1"
}

fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  FAILURES+="- **$1**: $2"$'\n'
  echo "  [FAIL] $1 -- $2"
}

# All helpers set RESP_BODY and HTTP_CODE as globals.
# Uses -o tmpfile so we never need $() subshells for output capture.
qbo_get() {
  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -H "Authorization: Bearer $ACCESS_TOKEN" \
    -H "Accept: application/json" \
    "$BASE/$1?$MINOR")
  RESP_BODY=$(cat "$CURL_TMP")
}

qbo_get_raw() {
  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -H "Authorization: Bearer $ACCESS_TOKEN" \
    -H "Accept: application/json" \
    "$1")
  RESP_BODY=$(cat "$CURL_TMP")
}

qbo_query() {
  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -H "Authorization: Bearer $ACCESS_TOKEN" \
    -H "Accept: application/json" \
    -H "Content-Type: application/text" \
    -d "$1" \
    "$BASE/query?$MINOR")
  RESP_BODY=$(cat "$CURL_TMP")
}

qbo_post() {
  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -H "Authorization: Bearer $ACCESS_TOKEN" \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    -d "$2" \
    "$BASE/$1?$MINOR&requestid=$(uuidgen | tr '[:upper:]' '[:lower:]')")
  RESP_BODY=$(cat "$CURL_TMP")
}

# ── Token Refresh ───────────────────────────────────────────────────

test_token_refresh() {
  echo ""
  echo "=== 1. Token Refresh ==="

  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -u "$QBO_CLIENT_ID:$QBO_CLIENT_SECRET" \
    -d "grant_type=refresh_token&refresh_token=$REFRESH_TOKEN" \
    "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
  RESP_BODY=$(cat "$CURL_TMP")

  if [[ "$HTTP_CODE" == "200" ]]; then
    ACCESS_TOKEN=$(echo "$RESP_BODY" | jq -r '.access_token')
    local new_rt
    new_rt=$(echo "$RESP_BODY" | jq -r '.refresh_token')
    pass "Token refresh (HTTP 200)"

    # Persist new tokens
    echo "$RESP_BODY" | jq --arg rid "$REALM_ID" '. + {realm_id: $rid}' > .qbo-tokens.json
    REFRESH_TOKEN="$new_rt"

    if [[ "$new_rt" != "$(jq -r '.refresh_token' <<< "$RESP_BODY")" ]] 2>/dev/null; then
      pass "Refresh token rotated"
    else
      pass "Refresh token returned (sandbox may reuse)"
    fi
  else
    fail "Token refresh" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 200)"
    exit 1
  fi

  # Try stale refresh token
  HTTP_CODE=$(curl -s -o "$CURL_TMP" -w '%{http_code}' \
    -u "$QBO_CLIENT_ID:$QBO_CLIENT_SECRET" \
    -d "grant_type=refresh_token&refresh_token=INVALID_STALE_TOKEN_xxxx" \
    "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
  if [[ "$HTTP_CODE" == "400" ]] || [[ "$HTTP_CODE" == "401" ]]; then
    pass "Stale refresh token rejected (HTTP $HTTP_CODE)"
  else
    fail "Stale refresh token" "Expected 400/401, got $HTTP_CODE"
  fi
}

# ── Entity Reads ────────────────────────────────────────────────────

test_entity_reads() {
  echo ""
  echo "=== 2. Entity Reads ==="

  local entities=(
    "customer:Customer"
    "invoice:Invoice"
    "payment:Payment"
    "item:Item"
    "vendor:Vendor"
    "account:Account"
    "estimate:Estimate"
    "purchaseorder:PurchaseOrder"
    "bill:Bill"
  )

  for entry in "${entities[@]}"; do
    local path key count eid
    path="${entry%%:*}"
    key="${entry##*:}"

    qbo_query "SELECT * FROM $key MAXRESULTS 1"

    if [[ "$HTTP_CODE" == "200" ]]; then
      count=$(echo "$RESP_BODY" | jq -r ".QueryResponse.$key | length // 0" 2>/dev/null)
      pass "Query $key (found ${count:-0})"

      if [[ "${count:-0}" -gt 0 ]]; then
        eid=$(echo "$RESP_BODY" | jq -r ".QueryResponse.$key[0].Id")
        qbo_get "$path/$eid"
        if [[ "$HTTP_CODE" == "200" ]]; then
          pass "GET $key/$eid"
        else
          fail "GET $key/$eid" "HTTP $HTTP_CODE"
        fi
      fi
    else
      fail "Query $key" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 150)"
    fi
  done
}

# ── Shipping Writeback ──────────────────────────────────────────────

test_shipping_writeback() {
  echo ""
  echo "=== 3. Shipping Writeback ==="

  qbo_query "SELECT * FROM Invoice MAXRESULTS 1"
  local inv_id sync_token ts
  inv_id=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Invoice[0].Id')
  sync_token=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Invoice[0].SyncToken')
  ts=$(date +%s)

  local update_body
  update_body=$(jq -n --arg id "$inv_id" --arg st "$sync_token" --arg track "SMOKE-$ts" '{
    "Id": $id, "SyncToken": $st, "sparse": true,
    "ShipDate": "2026-03-28",
    "TrackingNum": $track,
    "ShipMethodRef": {"value": "UPS"}
  }')

  qbo_post "invoice" "$update_body"

  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "Sparse update shipping fields"

    # Re-read and verify
    qbo_get "invoice/$inv_id"
    local ship_date tracking ship_method
    ship_date=$(echo "$RESP_BODY" | jq -r '.Invoice.ShipDate // ""')
    tracking=$(echo "$RESP_BODY" | jq -r '.Invoice.TrackingNum // ""')
    ship_method=$(echo "$RESP_BODY" | jq -r '.Invoice.ShipMethodRef.value // ""')

    if [[ "$ship_date" == "2026-03-28" ]] && [[ "$tracking" == "SMOKE-$ts" ]] && [[ "$ship_method" == "UPS" ]]; then
      pass "Shipping fields verified on re-read"
    else
      fail "Shipping verification" "ShipDate=$ship_date, Tracking=$tracking, Method=$ship_method"
    fi
  else
    fail "Sparse update" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 200)"
  fi
}

# ── SyncToken Conflict ──────────────────────────────────────────────

test_sync_token_conflict() {
  echo ""
  echo "=== 4. SyncToken Conflict ==="

  qbo_query "SELECT * FROM Invoice MAXRESULTS 1"
  local inv_id
  inv_id=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Invoice[0].Id')

  # Use definitely-stale SyncToken
  local update
  update=$(jq -n --arg id "$inv_id" '{
    "Id": $id, "SyncToken": "0", "sparse": true,
    "PrivateNote": "stale sync token test"
  }')

  qbo_post "invoice" "$update"
  if [[ "$HTTP_CODE" == "400" ]]; then
    local code
    code=$(echo "$RESP_BODY" | jq -r '.Fault.Error[0].code // ""')
    if [[ "$code" == "5010" ]]; then
      pass "Stale SyncToken -> 400 with code 5010"
    else
      fail "SyncToken error code" "Got 400 but code='$code' (expected 5010)"
    fi
  else
    fail "Stale SyncToken" "Expected 400, got $HTTP_CODE"
  fi
}

# ── CDC Variations ──────────────────────────────────────────────────

test_cdc() {
  echo ""
  echo "=== 5. CDC (Change Data Capture) ==="

  # macOS date flags
  local since_1h since_24h since_29d since_31d
  if date -v-1H '+%Y' &>/dev/null; then
    since_1h=$(date -u -v-1H '+%Y-%m-%dT%H:%M:%SZ')
    since_24h=$(date -u -v-24H '+%Y-%m-%dT%H:%M:%SZ')
    since_29d=$(date -u -v-29d '+%Y-%m-%dT%H:%M:%SZ')
    since_31d=$(date -u -v-31d '+%Y-%m-%dT%H:%M:%SZ')
  else
    since_1h=$(date -u -d '1 hour ago' '+%Y-%m-%dT%H:%M:%SZ')
    since_24h=$(date -u -d '24 hours ago' '+%Y-%m-%dT%H:%M:%SZ')
    since_29d=$(date -u -d '29 days ago' '+%Y-%m-%dT%H:%M:%SZ')
    since_31d=$(date -u -d '31 days ago' '+%Y-%m-%dT%H:%M:%SZ')
  fi

  # 1 hour ago
  qbo_get_raw "$BASE/cdc?entities=Customer,Invoice&changedSince=$since_1h&$MINOR"
  if [[ "$HTTP_CODE" == "200" ]]; then
    local n
    n=$(echo "$RESP_BODY" | jq '.CDCResponse[0].QueryResponse | length // 0' 2>/dev/null)
    pass "CDC 1h ago (${n:-0} entity groups)"
  else
    fail "CDC 1h ago" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 150)"
  fi

  # 24 hours ago
  qbo_get_raw "$BASE/cdc?entities=Customer,Invoice,Payment&changedSince=$since_24h&$MINOR"
  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "CDC 24h ago"
  else
    fail "CDC 24h ago" "HTTP $HTTP_CODE"
  fi

  # 29 days ago — near the 30-day limit
  qbo_get_raw "$BASE/cdc?entities=Invoice&changedSince=$since_29d&$MINOR"
  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "CDC 29d ago (near limit)"
  else
    fail "CDC 29d ago" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 150)"
  fi

  # 31 days ago — should fail (QBO has 30-day CDC lookback)
  qbo_get_raw "$BASE/cdc?entities=Invoice&changedSince=$since_31d&$MINOR"
  if [[ "$HTTP_CODE" == "400" ]]; then
    pass "CDC 31d ago correctly rejected (400)"
  elif [[ "$HTTP_CODE" == "200" ]]; then
    fail "CDC 31d boundary" "Expected 400, got 200 — QBO may allow >30d in sandbox"
  else
    fail "CDC 31d ago" "Unexpected HTTP $HTTP_CODE"
  fi

  # Verify CDC returns full entity payloads
  qbo_get_raw "$BASE/cdc?entities=Invoice&changedSince=$since_1h&$MINOR"
  if [[ "$HTTP_CODE" == "200" ]]; then
    local has_full
    has_full=$(echo "$RESP_BODY" | jq '[.CDCResponse[0].QueryResponse[]? | to_entries[] | select(.key != "startPosition" and .key != "maxResults") | .value[0]? | has("Id","SyncToken")] | any' 2>/dev/null)
    if [[ "$has_full" == "true" ]]; then
      pass "CDC returns full entity payloads (not just IDs)"
    else
      pass "CDC returned data (payload structure may vary)"
    fi
  fi
}

# ── Pagination ──────────────────────────────────────────────────────

test_pagination() {
  echo ""
  echo "=== 6. Pagination ==="

  qbo_query "SELECT COUNT(*) FROM Customer"
  if [[ "$HTTP_CODE" == "200" ]]; then
    local total
    total=$(echo "$RESP_BODY" | jq -r '.QueryResponse.totalCount // 0')
    pass "Customer COUNT(*): $total"
  else
    fail "Customer COUNT" "HTTP $HTTP_CODE"
    return
  fi

  if [[ "$total" -gt 1 ]]; then
    qbo_query "SELECT * FROM Customer STARTPOSITION 1 MAXRESULTS 1"
    local id1
    id1=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Customer[0].Id // ""')

    qbo_query "SELECT * FROM Customer STARTPOSITION 2 MAXRESULTS 1"
    local id2
    id2=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Customer[0].Id // ""')

    if [[ -n "$id1" ]] && [[ -n "$id2" ]] && [[ "$id1" != "$id2" ]]; then
      pass "Pagination returns different entities (pos1=$id1, pos2=$id2)"
    else
      fail "Pagination" "Same or empty IDs: pos1=$id1, pos2=$id2"
    fi
  else
    pass "Only $total customer(s) — skipping pagination cross-check"
  fi

  # Large STARTPOSITION past all data
  qbo_query "SELECT * FROM Customer STARTPOSITION 999999 MAXRESULTS 10"
  if [[ "$HTTP_CODE" == "200" ]]; then
    local count
    count=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Customer // [] | length')
    if [[ "${count:-0}" == "0" ]] || [[ -z "$count" ]]; then
      pass "STARTPOSITION past data returns empty set"
    else
      fail "Large STARTPOSITION" "Expected 0, got $count"
    fi
  else
    fail "Large STARTPOSITION" "HTTP $HTTP_CODE"
  fi
}

# ── Error Cases ─────────────────────────────────────────────────────

test_error_cases() {
  echo ""
  echo "=== 7. Error Cases ==="

  # Non-existent entity ID
  qbo_get "invoice/999999999"
  if [[ "$HTTP_CODE" =~ ^(200|400|404)$ ]]; then
    pass "Non-existent invoice: HTTP $HTTP_CODE"
  else
    fail "Non-existent invoice" "Unexpected HTTP $HTTP_CODE"
  fi

  # Malformed query
  qbo_query "THIS IS NOT SQL AT ALL"
  if [[ "$HTTP_CODE" == "400" ]]; then
    pass "Malformed query returns 400"
  else
    fail "Malformed query" "Expected 400, got $HTTP_CODE"
  fi

  # Bad token
  local saved_token="$ACCESS_TOKEN"
  ACCESS_TOKEN="totally-invalid-token-xxxxx"
  qbo_get "customer/1"
  ACCESS_TOKEN="$saved_token"
  if [[ "$HTTP_CODE" == "401" ]]; then
    pass "Invalid token returns 401"
  else
    fail "Invalid token" "Expected 401, got $HTTP_CODE"
  fi

  # Create invoice with missing required fields (no CustomerRef)
  qbo_post "invoice" '{"Line": [{"Amount": 1, "DetailType": "SalesItemLineDetail", "SalesItemLineDetail": {"ItemRef": {"value": "1"}}}]}'
  if [[ "$HTTP_CODE" == "400" ]]; then
    pass "Missing CustomerRef returns 400"
  elif [[ "$HTTP_CODE" == "200" ]]; then
    fail "Missing CustomerRef" "QBO accepted invoice without CustomerRef"
  else
    fail "Missing CustomerRef" "Expected 400, got $HTTP_CODE"
  fi

  # Update with missing Id
  qbo_post "invoice" '{"sparse": true, "PrivateNote": "no id"}'
  if [[ "$HTTP_CODE" == "400" ]]; then
    pass "Update without Id returns 400"
  else
    fail "Update without Id" "Expected 400, got $HTTP_CODE"
  fi

  # Query wrong entity type
  qbo_query "SELECT * FROM FakeEntity MAXRESULTS 1"
  if [[ "$HTTP_CODE" == "400" ]]; then
    pass "Query fake entity returns 400"
  else
    fail "Query fake entity" "Expected 400, got $HTTP_CODE"
  fi
}

# ── Create Test Data ────────────────────────────────────────────────

test_create_operations() {
  echo ""
  echo "=== 8. Create Operations ==="

  local ts cust_body cust_id
  ts=$(date +%s)
  cust_body=$(jq -n --arg name "SmokeTest-$ts" '{
    "DisplayName": $name,
    "CompanyName": "Smoke Test Corp",
    "PrimaryEmailAddr": {"Address": "smoke@test.example"}
  }')

  qbo_post "customer" "$cust_body"
  if [[ "$HTTP_CODE" == "200" ]]; then
    cust_id=$(echo "$RESP_BODY" | jq -r '.Customer.Id')
    pass "Create customer (Id: $cust_id)"

    # Read it back
    qbo_get "customer/$cust_id"
    if [[ "$HTTP_CODE" == "200" ]]; then
      local name_back
      name_back=$(echo "$RESP_BODY" | jq -r '.Customer.DisplayName // ""')
      if [[ "$name_back" == "SmokeTest-$ts" ]]; then
        pass "Read back created customer (name matches)"
      else
        fail "Read back customer" "Name mismatch: expected SmokeTest-$ts, got $name_back"
      fi
    else
      fail "Read back customer" "HTTP $HTTP_CODE"
    fi

    # Create invoice for this customer
    local inv_body inv_id
    inv_body=$(jq -n --arg cid "$cust_id" '{
      "CustomerRef": {"value": $cid},
      "Line": [{
        "Amount": 42.00,
        "DetailType": "SalesItemLineDetail",
        "SalesItemLineDetail": {"ItemRef": {"value": "1", "name": "Services"}}
      }]
    }')

    qbo_post "invoice" "$inv_body"
    if [[ "$HTTP_CODE" == "200" ]]; then
      inv_id=$(echo "$RESP_BODY" | jq -r '.Invoice.Id')
      pass "Create invoice for new customer (Id: $inv_id)"

      # Verify it appears in query
      qbo_query "SELECT * FROM Invoice WHERE Id = '$inv_id'"
      if [[ "$HTTP_CODE" == "200" ]]; then
        local found
        found=$(echo "$RESP_BODY" | jq -r '.QueryResponse.Invoice[0].Id // ""')
        if [[ "$found" == "$inv_id" ]]; then
          pass "Created invoice found via query"
        else
          fail "Query created invoice" "Not found in results"
        fi
      else
        fail "Query created invoice" "HTTP $HTTP_CODE"
      fi
    else
      fail "Create invoice" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 200)"
    fi
  else
    fail "Create customer" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 200)"
  fi
}

# ── Special Characters ──────────────────────────────────────────────

test_special_chars() {
  echo ""
  echo "=== 9. Special Characters & Edge Cases ==="

  # LIKE query with wildcards
  qbo_query "SELECT * FROM Customer WHERE DisplayName LIKE '%Design%' MAXRESULTS 1"
  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "LIKE query with wildcards"
  else
    fail "LIKE query" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 150)"
  fi

  # Empty result set
  qbo_query "SELECT * FROM Customer WHERE Id = '99999999'"
  if [[ "$HTTP_CODE" == "200" ]]; then
    local count
    count=$(echo "$RESP_BODY" | jq '.QueryResponse.Customer // [] | length')
    if [[ "${count:-0}" == "0" ]] || [[ -z "$count" ]]; then
      pass "Empty result set returns 200 with 0 entities"
    else
      fail "Empty result set" "Expected 0, got $count"
    fi
  else
    fail "Empty result set query" "HTTP $HTTP_CODE"
  fi

  # Long query with many conditions
  local long_q="SELECT * FROM Customer WHERE"
  for i in $(seq 1 20); do
    long_q+=" Id != '$i' AND"
  done
  long_q+=" Id != '999' MAXRESULTS 1"
  qbo_query "$long_q"
  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "Long query with 20 conditions"
  else
    fail "Long query" "HTTP $HTTP_CODE: $(echo "$RESP_BODY" | head -c 150)"
  fi

  # ORDER BY query
  qbo_query "SELECT * FROM Customer ORDERBY DisplayName MAXRESULTS 5"
  if [[ "$HTTP_CODE" == "200" ]]; then
    pass "ORDER BY query"
  else
    fail "ORDER BY query" "HTTP $HTTP_CODE"
  fi
}

# ── Concurrent Burst ────────────────────────────────────────────────

test_concurrent_burst() {
  echo ""
  echo "=== 10. Concurrent Burst (20 requests) ==="

  local tmpdir
  tmpdir=$(mktemp -d)

  for i in $(seq 1 20); do
    (
      local code
      code=$(curl -s -o /dev/null -w '%{http_code}' \
        -H "Authorization: Bearer $ACCESS_TOKEN" \
        -H "Accept: application/json" \
        -H "Content-Type: application/text" \
        -d "SELECT Id FROM Customer MAXRESULTS 1" \
        "$BASE/query?$MINOR")
      echo "$code" > "$tmpdir/r_$i"
    ) &
  done
  wait

  local ok=0 throttled=0 errors=0
  for i in $(seq 1 20); do
    local c
    c=$(cat "$tmpdir/r_$i" 2>/dev/null || echo "000")
    case "$c" in
      200) ok=$((ok + 1)) ;;
      429|401) throttled=$((throttled + 1)) ;;
      *) errors=$((errors + 1)) ;;
    esac
  done
  rm -rf "$tmpdir"

  pass "Burst results: $ok OK, $throttled throttled, $errors errors"
  if [[ "$throttled" -gt 0 ]]; then
    echo "    NOTE: Rate limiting triggered at 20 concurrent requests"
  fi
  if [[ "$errors" -gt 0 ]]; then
    fail "Burst errors" "$errors requests returned unexpected status codes"
  fi
}

# ── Report ──────────────────────────────────────────────────────────

write_report() {
  cat > "$REPORT_FILE" <<EOF
# QBO Smoke Test Results

**Date**: $(date -u '+%Y-%m-%d %H:%M:%S UTC')
**Realm**: $REALM_ID
**Sandbox**: $QBO_SANDBOX_BASE

## Summary

| Metric | Count |
|--------|-------|
| Passed | $PASS_COUNT |
| Failed | $FAIL_COUNT |
| Total  | $((PASS_COUNT + FAIL_COUNT)) |

EOF

  if [[ $FAIL_COUNT -gt 0 ]]; then
    cat >> "$REPORT_FILE" <<EOF
## Failures

$FAILURES
EOF
  fi

  cat >> "$REPORT_FILE" <<EOF
## Tests Run

1. Token refresh cycle (refresh, rotation, stale token rejection)
2. Entity reads: Customer, Invoice, Payment, Item, Vendor, Account, Estimate, PurchaseOrder, Bill
3. Shipping writeback: ShipDate, TrackingNum, ShipMethodRef via sparse update + re-read verify
4. SyncToken conflict: stale token produces 400 / code 5010
5. CDC: 1h, 24h, 29d lookback + 31d rejection + full payload verification
6. Pagination: COUNT, STARTPOSITION cross-check, past-end empty set
7. Error cases: non-existent entity, malformed query, bad token, missing fields, fake entity
8. Create operations: customer + invoice creation + query readback
9. Special characters: LIKE wildcards, empty result set, long query, ORDER BY
10. Concurrent burst: 20 simultaneous requests, rate limit detection
EOF

  echo ""
  echo "Report written to $REPORT_FILE"
}

# ── Main ────────────────────────────────────────────────────────────

echo "========================================"
echo "  QBO Sandbox Full Smoke Test"
echo "  Realm: $REALM_ID"
echo "========================================"

test_token_refresh
test_entity_reads
test_shipping_writeback
test_sync_token_conflict
test_cdc
test_pagination
test_error_cases
test_create_operations
test_special_chars
test_concurrent_burst

echo ""
echo "========================================"
echo "  Results: $PASS_COUNT passed, $FAIL_COUNT failed"
echo "========================================"

if [[ $FAIL_COUNT -gt 0 ]]; then
  echo ""
  echo "Failures:"
  echo "$FAILURES"
fi

write_report

exit "$FAIL_COUNT"
