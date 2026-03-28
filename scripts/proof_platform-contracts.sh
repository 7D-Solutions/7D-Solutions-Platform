#!/usr/bin/env bash
# Proof script for platform/platform-contracts
# Run this before bumping to v1.0.0 or promoting to production.
# Usage: ./scripts/proof_platform-contracts.sh
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
echo "  Proof: platform-contracts"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p platform_contracts 2>&1; then
  log_pass "cargo build -p platform_contracts"
else
  log_fail "cargo build -p platform_contracts"
fi

# ── Gate 2: Unit tests ───────────────────────────────────────────────────────
log_step "Unit tests"
if ./scripts/cargo-slot.sh test -p platform_contracts 2>&1; then
  log_pass "cargo test -p platform_contracts"
else
  log_fail "cargo test -p platform_contracts"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  platform-contracts proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
