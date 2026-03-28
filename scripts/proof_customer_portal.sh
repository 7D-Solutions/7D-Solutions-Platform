#!/usr/bin/env bash
# Proof script for modules/customer-portal (package: customer-portal)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_customer_portal.sh
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
echo "  Proof: customer-portal"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p customer-portal 2>&1; then
  log_pass "cargo build -p customer-portal"
else
  log_fail "cargo build -p customer-portal"
fi

# ── Gate 2: Clippy (no warnings) ────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p customer-portal -- -D warnings 2>&1; then
  log_pass "cargo clippy -p customer-portal (no warnings)"
else
  log_fail "cargo clippy -p customer-portal"
fi

# ── Gate 3: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p customer-portal 2>&1; then
  log_pass "cargo test -p customer-portal"
else
  log_fail "cargo test -p customer-portal"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  customer-portal proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
