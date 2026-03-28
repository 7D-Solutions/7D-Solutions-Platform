#!/usr/bin/env bash
# Proof script for modules/shipping-receiving
# Run this before bumping to v1.0.0 or promoting to production.
# Usage: ./scripts/proof_shipping-receiving.sh
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

PASS=0
FAIL=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

echo "=============================="
echo "  Proof: shipping-receiving"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p shipping-receiving-rs 2>&1; then
  log_pass "cargo build -p shipping-receiving-rs"
else
  log_fail "cargo build -p shipping-receiving-rs"
fi

# ── Gate 2: Unit tests ───────────────────────────────────────────────────────
log_step "Unit tests"
if ./scripts/cargo-slot.sh test -p shipping-receiving-rs 2>&1; then
  log_pass "cargo test -p shipping-receiving-rs"
else
  log_fail "cargo test -p shipping-receiving-rs"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  shipping-receiving proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
