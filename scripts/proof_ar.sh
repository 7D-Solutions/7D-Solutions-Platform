#!/usr/bin/env bash
# Proof script for modules/ar (package: ar-rs)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_ar.sh                          # local build + unit tests only
#   ./scripts/proof_ar.sh --staging <host>         # + staging health + payment loop idempotency
#   ./scripts/proof_ar.sh --staging <host> --secret <secret>  # full proof
#
# Environment variables (override CLI flags):
#   STAGING_HOST              — VPS hostname or IP
#   TILLED_WEBHOOK_SECRET     — Tilled HMAC-SHA256 webhook signing secret
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST="${STAGING_HOST:-}"
WEBHOOK_SECRET="${TILLED_WEBHOOK_SECRET:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staging) STAGING_HOST="$2"; shift 2 ;;
    --secret)  WEBHOOK_SECRET="$2"; shift 2 ;;
    *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

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

# ── Gate 3: Staging health check ─────────────────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  AR_PORT="${AR_PORT:-8086}"
  AR_BASE="http://${STAGING_HOST}:${AR_PORT}"

  log_step "Staging health (${AR_BASE})"
  if curl --silent --fail --max-time 10 "${AR_BASE}/healthz" > /dev/null 2>&1; then
    log_pass "GET ${AR_BASE}/healthz → 200"
  else
    log_fail "GET ${AR_BASE}/healthz did not return 200"
  fi

  if curl --silent --fail --max-time 10 "${AR_BASE}/api/ready" > /dev/null 2>&1; then
    log_pass "GET ${AR_BASE}/api/ready → 200"
  else
    log_fail "GET ${AR_BASE}/api/ready did not return 200"
  fi

  # ── Gate 4: Payment loop + webhook replay idempotency ────────────────────
  if [[ -n "$WEBHOOK_SECRET" ]]; then
    log_step "Payment loop + webhook replay idempotency (staging)"
    if STAGING_HOST="$STAGING_HOST" TILLED_WEBHOOK_SECRET="$WEBHOOK_SECRET" \
       bash ./scripts/staging/payment_loop.sh 2>&1; then
      log_pass "Payment loop: invoice → webhook → posting → replay idempotency PROVEN"
    else
      log_fail "Payment loop FAILED — check scripts/staging/payment_loop.sh output"
    fi
  else
    log_pass "Webhook idempotency skipped — TILLED_WEBHOOK_SECRET not set (set --secret to run full proof)"
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
