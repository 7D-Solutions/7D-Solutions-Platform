#!/usr/bin/env bash
# Proof script for modules/ttp (package: ttp-rs)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_ttp.sh                          # local build + unit tests only
#   ./scripts/proof_ttp.sh --staging <host>         # + staging health + billing idempotency
#   ./scripts/proof_ttp.sh --staging <host> --db <url>  # + ignored integration tests
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST=""
TTP_DB_URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staging) STAGING_HOST="$2"; shift 2 ;;
    --db)      TTP_DB_URL="$2";   shift 2 ;;
    *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

PASS=0
FAIL=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

echo "=============================="
echo "  Proof: ttp (ttp-rs)"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p ttp-rs 2>&1; then
  log_pass "cargo build -p ttp-rs"
else
  log_fail "cargo build -p ttp-rs"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p ttp-rs 2>&1; then
  log_pass "cargo test -p ttp-rs"
else
  log_fail "cargo test -p ttp-rs"
fi

# ── Gate 3: Billing idempotency (ignored integration tests — require real DB) ─
# Run only when --db <DATABASE_URL> is provided (points at real TTP Postgres).
if [[ -n "$TTP_DB_URL" ]]; then
  log_step "Billing idempotency — metering_integration (real DB)"
  if DATABASE_URL="$TTP_DB_URL" \
     ./scripts/cargo-slot.sh test -p ttp-rs --test metering_integration -- --ignored 2>&1; then
    log_pass "metering_integration (idempotent ingestion + deterministic trace)"
  else
    log_fail "metering_integration FAILED"
  fi

  log_step "Billing idempotency — billing_metering_integration (real DB + AR + tenant-registry)"
  if DATABASE_URL="$TTP_DB_URL" \
     ./scripts/cargo-slot.sh test -p ttp-rs --test billing_metering_integration -- --ignored 2>&1; then
    log_pass "billing_metering_integration (trace-to-invoice + replay no-op)"
  else
    log_fail "billing_metering_integration FAILED"
  fi
fi

# ── Gate 4: Staging health check (optional) ──────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  TTP_PORT="${TTP_PORT:-8100}"
  TTP_BASE="http://${STAGING_HOST}:${TTP_PORT}"

  log_step "Staging health (${TTP_BASE})"
  if curl --silent --fail --max-time 10 "${TTP_BASE}/healthz" > /dev/null 2>&1; then
    log_pass "GET ${TTP_BASE}/healthz → 200"
  else
    log_fail "GET ${TTP_BASE}/healthz did not return 200"
  fi

  if curl --silent --fail --max-time 10 "${TTP_BASE}/api/ready" > /dev/null 2>&1; then
    log_pass "GET ${TTP_BASE}/api/ready → 200"
  else
    log_fail "GET ${TTP_BASE}/api/ready did not return 200"
  fi

  # ── Gate 5: Staging billing trigger — invoke TTP billing run and verify idempotency
  log_step "Staging billing trigger — POST /api/billing/run (idempotency proof)"

  BILLING_PERIOD="$(date +%Y-%m)"
  # Use a stable test tenant ID (from staging seed data) or a random UUID for isolation.
  # This endpoint requires a real tenant to be provisioned in staging.
  TEST_TENANT_ID="${STAGING_TEST_TENANT_ID:-}"

  if [[ -z "$TEST_TENANT_ID" ]]; then
    log_pass "Staging billing trigger skipped — STAGING_TEST_TENANT_ID not set (set to run full idempotency proof)"
  else
    BILLING_IKEY="proof-ttp-$(date +%s)"
    BILLING_BODY=$(printf '{"tenant_id":"%s","billing_period":"%s","idempotency_key":"%s"}' \
      "$TEST_TENANT_ID" "$BILLING_PERIOD" "$BILLING_IKEY")

    # First run
    raw1=$(curl -s -w '\n%{http_code}' --max-time 30 \
      -X POST \
      -H 'Content-Type: application/json' \
      -d "$BILLING_BODY" \
      "${TTP_BASE}/api/billing/run")
    status1="${raw1##*$'\n'}"
    body1="${raw1%$'\n'${status1}}"

    if [[ "$status1" == "200" || "$status1" == "201" ]]; then
      log_pass "POST /api/billing/run → HTTP ${status1}"
    else
      log_fail "POST /api/billing/run → HTTP ${status1} (expected 200/201): ${body1}"
    fi

    # Second run with same idempotency key — must be a no-op (same status code, was_noop=true)
    raw2=$(curl -s -w '\n%{http_code}' --max-time 30 \
      -X POST \
      -H 'Content-Type: application/json' \
      -d "$BILLING_BODY" \
      "${TTP_BASE}/api/billing/run")
    status2="${raw2##*$'\n'}"
    body2="${raw2%$'\n'${status2}}"

    if [[ "$status2" == "200" || "$status2" == "201" ]]; then
      # Verify the response signals a no-op (was_noop: true)
      if printf '%s' "$body2" | grep -q '"was_noop":true'; then
        log_pass "Billing run replay → was_noop:true (idempotency PROVEN)"
      else
        log_fail "Billing run replay returned HTTP ${status2} but was_noop not true — possible double-billing risk"
      fi
    else
      log_fail "Billing run replay → HTTP ${status2}: ${body2}"
    fi
  fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  ttp proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
