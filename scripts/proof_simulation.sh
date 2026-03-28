#!/usr/bin/env bash
# Proof script for tools/simulation (package: simulation)
# Run this before promoting to v1.0.0.
#
# Usage:
#   ./scripts/proof_simulation.sh
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
echo "  Proof: simulation"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p simulation 2>&1; then
  log_pass "cargo build -p simulation"
else
  log_fail "cargo build -p simulation"
fi

# ── Gate 2: Clippy ───────────────────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p simulation 2>&1; then
  log_pass "cargo clippy -p simulation"
else
  log_fail "cargo clippy -p simulation"
fi

# ── Gate 3: Unit tests ───────────────────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p simulation 2>&1; then
  log_pass "cargo test -p simulation"
else
  log_fail "cargo test -p simulation"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  simulation proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
