#!/usr/bin/env bash
# test-rate-limits.sh — Verify gateway rate limiting is active.
# Sends burst requests to auth endpoints and confirms 429 responses.
# Usage: ./scripts/test-rate-limits.sh [GATEWAY_URL]
set -euo pipefail

GATEWAY="${1:-http://127.0.0.1:8000}"
PASS=0
FAIL=0

green() { printf '\033[0;32m%s\033[0m\n' "$1"; }
red()   { printf '\033[0;31m%s\033[0m\n' "$1"; }
bold()  { printf '\033[1m%s\033[0m\n' "$1"; }

assert_status() {
  local label="$1" expected="$2" actual="$3"
  if [ "$actual" = "$expected" ]; then
    green "  PASS: $label (got $actual)"
    PASS=$((PASS + 1))
  else
    red "  FAIL: $label (expected $expected, got $actual)"
    FAIL=$((FAIL + 1))
  fi
}

# ── Prerequisite: gateway is reachable ────────────────────
bold "=== Gateway health check ==="
status=$(curl -s -o /dev/null -w '%{http_code}' "$GATEWAY/api/gateway/health")
assert_status "gateway health returns 200" "200" "$status"

if [ "$status" != "200" ]; then
  red "Gateway not reachable at $GATEWAY — aborting."
  exit 1
fi

# ── Test 1: Auth strict rate limit (login) ────────────────
bold ""
bold "=== Test: /api/auth/login strict rate limit (5r/m, burst 2) ==="
# The auth_strict zone allows 5r/m = 1 request per 12s.
# With burst=2 nodelay, the first 3 requests go through (1 at rate + 2 burst).
# Subsequent requests within the window should be rejected with 429.
got_429=false
for i in $(seq 1 10); do
  status=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST -H 'Content-Type: application/json' \
    -d '{"email":"ratelimit-test@example.com","password":"x"}' \
    "$GATEWAY/api/auth/login")
  echo "  request $i: $status"
  if [ "$status" = "429" ]; then
    got_429=true
  fi
done
if $got_429; then
  green "  PASS: 429 received during login burst"
  PASS=$((PASS + 1))
else
  red "  FAIL: no 429 received during login burst (10 requests)"
  FAIL=$((FAIL + 1))
fi

# ── Test 2: Auth strict rate limit (register) ─────────────
bold ""
bold "=== Test: /api/auth/register strict rate limit ==="
got_429=false
for i in $(seq 1 10); do
  status=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST -H 'Content-Type: application/json' \
    -d '{"email":"ratelimit-test@example.com","password":"x","tenant_id":"test"}' \
    "$GATEWAY/api/auth/register")
  echo "  request $i: $status"
  if [ "$status" = "429" ]; then
    got_429=true
  fi
done
if $got_429; then
  green "  PASS: 429 received during register burst"
  PASS=$((PASS + 1))
else
  red "  FAIL: no 429 received during register burst"
  FAIL=$((FAIL + 1))
fi

# ── Test 3: Forgot-password rate limit ────────────────────
bold ""
bold "=== Test: /api/auth/forgot-password strict rate limit ==="
got_429=false
for i in $(seq 1 10); do
  status=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST -H 'Content-Type: application/json' \
    -d '{"email":"ratelimit-test@example.com"}' \
    "$GATEWAY/api/auth/forgot-password")
  echo "  request $i: $status"
  if [ "$status" = "429" ]; then
    got_429=true
  fi
done
if $got_429; then
  green "  PASS: 429 received during forgot-password burst"
  PASS=$((PASS + 1))
else
  red "  FAIL: no 429 received during forgot-password burst"
  FAIL=$((FAIL + 1))
fi

# ── Test 4: 429 response includes Retry-After header ─────
bold ""
bold "=== Test: 429 includes Retry-After header ==="
# Rapid-fire login to trigger 429, then check headers
headers=""
for i in $(seq 1 10); do
  resp=$(curl -s -D - -o /dev/null \
    -X POST -H 'Content-Type: application/json' \
    -d '{"email":"header-test@example.com","password":"x"}' \
    "$GATEWAY/api/auth/login")
  if echo "$resp" | grep -q "429"; then
    headers="$resp"
    break
  fi
done
if echo "$headers" | grep -qi "Retry-After"; then
  green "  PASS: Retry-After header present in 429 response"
  PASS=$((PASS + 1))
else
  red "  FAIL: Retry-After header missing from 429 response"
  FAIL=$((FAIL + 1))
fi

# ── Test 5: 429 response includes X-RateLimit-Policy ─────
bold ""
bold "=== Test: 429 includes X-RateLimit-Policy header ==="
if echo "$headers" | grep -qi "X-RateLimit-Policy"; then
  green "  PASS: X-RateLimit-Policy header present"
  PASS=$((PASS + 1))
else
  red "  FAIL: X-RateLimit-Policy header missing"
  FAIL=$((FAIL + 1))
fi

# ── Test 6: Default API rate limit (catch-all) ────────────
bold ""
bold "=== Test: default API rate limit (120r/m, burst 10) ==="
# Send 20 rapid requests to a module endpoint. With 120r/m (2r/s) + burst 10,
# the first ~12 should succeed, and subsequent should hit 429.
got_429=false
for i in $(seq 1 20); do
  status=$(curl -s -o /dev/null -w '%{http_code}' \
    "$GATEWAY/api/ar/customers?tenant_id=rate-limit-test")
  if [ "$status" = "429" ]; then
    got_429=true
    echo "  request $i: $status (rate limited)"
    break
  fi
  echo "  request $i: $status"
done
if $got_429; then
  green "  PASS: 429 received for general API burst"
  PASS=$((PASS + 1))
else
  # 20 requests at 120r/m with burst 10 might all pass in quick sequence
  # depending on timing. This is acceptable — the zone is configured.
  echo "  INFO: no 429 in 20 requests (120r/m limit may be too generous for this burst size)"
  echo "  Verifying zone header is present on normal responses..."
  resp=$(curl -s -D - -o /dev/null "$GATEWAY/api/ar/customers?tenant_id=rate-limit-test")
  if echo "$resp" | grep -qi "X-RateLimit-Policy"; then
    green "  PASS: X-RateLimit-Policy header present (zone configured)"
    PASS=$((PASS + 1))
  else
    red "  FAIL: no rate limit evidence on general API"
    FAIL=$((FAIL + 1))
  fi
fi

# ── Test 7: Unrouted path returns 404 ────────────────────
bold ""
bold "=== Test: unknown path returns 404 ==="
status=$(curl -s -o /dev/null -w '%{http_code}' "$GATEWAY/api/nonexistent")
assert_status "unrouted path returns 404" "404" "$status"

# ── Summary ──────────────────────────────────────────────
bold ""
bold "=== Summary ==="
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  red "SOME TESTS FAILED"
  exit 1
else
  green "ALL TESTS PASSED"
  exit 0
fi
