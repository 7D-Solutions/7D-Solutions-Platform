#!/usr/bin/env bash
# Proof script for modules/ar (package: ar-rs)
# Run this before bumping to v1.0.0 or promoting to production.
# Usage: ./scripts/proof_ar.sh [--staging <host>]
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST="${2:-}"
PASS=0
FAIL=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

echo "=============================="
echo "  Proof: ar (ar-rs)"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p ar-rs 2>&1; then
  log_pass "cargo build -p ar-rs"
else
  log_fail "cargo build -p ar-rs"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p ar-rs 2>&1; then
  log_pass "cargo test -p ar-rs"
else
  log_fail "cargo test -p ar-rs"
fi

# ── Gate 3: Staging health check (optional) ──────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  log_step "Staging health (${STAGING_HOST})"
  HEALTH_URL="http://${STAGING_HOST}/healthz"
  if curl --silent --fail --max-time 10 "$HEALTH_URL" > /dev/null 2>&1; then
    log_pass "GET $HEALTH_URL → 200"
  else
    log_fail "GET $HEALTH_URL did not return 200"
  fi
  READY_URL="http://${STAGING_HOST}/api/ready"
  if curl --silent --fail --max-time 10 "$READY_URL" > /dev/null 2>&1; then
    log_pass "GET $READY_URL → 200"
  else
    log_fail "GET $READY_URL did not return 200"
  fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  ar proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
