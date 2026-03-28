#!/usr/bin/env bash
# Proof script for platform/tax-core (package: tax-core)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_tax_core.sh
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
echo "  Proof: tax-core"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p tax-core 2>&1; then
  log_pass "cargo build -p tax-core"
else
  log_fail "cargo build -p tax-core"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p tax-core 2>&1; then
  log_pass "cargo test -p tax-core"
else
  log_fail "cargo test -p tax-core"
fi

# ── Gate 3: Clippy ───────────────────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p tax-core -- -D warnings 2>&1; then
  log_pass "clippy -p tax-core (zero warnings)"
else
  log_fail "clippy -p tax-core"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  tax-core proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
