#!/usr/bin/env bash
# Proof script for platform/tenant-registry
# Run this before bumping to v1.0.0 or promoting to production.
# Usage: ./scripts/proof_tenant_registry.sh [--staging <host>]
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST="${2:-}"
TR_PORT="${TENANT_REGISTRY_PORT:-8092}"
PASS=0
FAIL=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

echo "=============================="
echo "  Proof: tenant-registry"
echo "=============================="

# ── Gate 1: Build ─────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p tenant-registry 2>&1; then
  log_pass "cargo build -p tenant-registry"
else
  log_fail "cargo build -p tenant-registry"
fi

# ── Gate 2: Unit + integration tests ──────────────────────────────────────────
log_step "Unit + integration tests"
if ./scripts/cargo-slot.sh test -p tenant-registry 2>&1; then
  log_pass "cargo test -p tenant-registry"
else
  log_fail "cargo test -p tenant-registry"
fi

# ── Gate 3: Staging checks (optional) ─────────────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  log_step "Staging health (${STAGING_HOST})"

  HEALTH_URL="http://${STAGING_HOST}:${TR_PORT}/healthz"
  if curl --silent --fail --max-time 10 "$HEALTH_URL" > /dev/null 2>&1; then
    log_pass "GET $HEALTH_URL → 200"
  else
    log_fail "GET $HEALTH_URL did not return 200"
  fi

  READY_URL="http://${STAGING_HOST}:${TR_PORT}/api/ready"
  if curl --silent --fail --max-time 10 "$READY_URL" > /dev/null 2>&1; then
    log_pass "GET $READY_URL → 200"
  else
    log_fail "GET $READY_URL did not return 200"
  fi

  # Tenant list route — used by TCP UI
  LIST_URL="http://${STAGING_HOST}:${TR_PORT}/api/tenants"
  TENANT_LIST_STATUS=$(curl --silent --output /dev/null --write-out "%{http_code}" --max-time 10 "$LIST_URL" 2>/dev/null || echo "000")
  if [[ "$TENANT_LIST_STATUS" == "200" ]]; then
    log_pass "GET $LIST_URL → 200"
  else
    log_fail "GET $LIST_URL → $TENANT_LIST_STATUS (expected 200)"
  fi

  # Plan catalog route — used by control-plane + TTP
  PLANS_URL="http://${STAGING_HOST}:${TR_PORT}/api/plans"
  PLANS_STATUS=$(curl --silent --output /dev/null --write-out "%{http_code}" --max-time 10 "$PLANS_URL" 2>/dev/null || echo "000")
  if [[ "$PLANS_STATUS" == "200" ]]; then
    log_pass "GET $PLANS_URL → 200"
  else
    log_fail "GET $PLANS_URL → $PLANS_STATUS (expected 200)"
  fi

  # Non-existent tenant returns 404 (not 500) — DB migration safety check
  FAKE_TENANT="00000000-0000-0000-0000-000000000000"
  DETAIL_STATUS=$(curl --silent --output /dev/null --write-out "%{http_code}" --max-time 10 \
    "http://${STAGING_HOST}:${TR_PORT}/api/tenants/${FAKE_TENANT}" 2>/dev/null || echo "000")
  if [[ "$DETAIL_STATUS" == "404" ]]; then
    log_pass "GET /api/tenants/{unknown} → 404 (migration safety: schema correct)"
  else
    log_fail "GET /api/tenants/{unknown} → $DETAIL_STATUS (expected 404 — possible schema migration issue)"
  fi

  # app-id route returns 404 for unknown tenant (used by control-plane + TTP)
  APPID_STATUS=$(curl --silent --output /dev/null --write-out "%{http_code}" --max-time 10 \
    "http://${STAGING_HOST}:${TR_PORT}/api/tenants/${FAKE_TENANT}/app-id" 2>/dev/null || echo "000")
  if [[ "$APPID_STATUS" == "404" ]]; then
    log_pass "GET /api/tenants/{unknown}/app-id → 404 (app_id routing correct)"
  else
    log_fail "GET /api/tenants/{unknown}/app-id → $APPID_STATUS (expected 404)"
  fi

  # entitlements route returns 404 for unknown tenant (used by identity-auth)
  ENT_STATUS=$(curl --silent --output /dev/null --write-out "%{http_code}" --max-time 10 \
    "http://${STAGING_HOST}:${TR_PORT}/api/tenants/${FAKE_TENANT}/entitlements" 2>/dev/null || echo "000")
  if [[ "$ENT_STATUS" == "404" ]]; then
    log_pass "GET /api/tenants/{unknown}/entitlements → 404 (entitlement routing correct)"
  else
    log_fail "GET /api/tenants/{unknown}/entitlements → $ENT_STATUS (expected 404)"
  fi
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  tenant-registry proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
