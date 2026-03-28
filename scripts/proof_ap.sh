#!/usr/bin/env bash
# Proof script for modules/ap (package: ap)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_ap.sh
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
echo "  Proof: ap"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p ap 2>&1; then
  log_pass "cargo build -p ap"
else
  log_fail "cargo build -p ap"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p ap 2>&1; then
  log_pass "cargo test -p ap"
else
  log_fail "cargo test -p ap"
fi

# ── Gate 3: Clippy ───────────────────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p ap -- -D warnings 2>&1; then
  log_pass "clippy -p ap (zero warnings)"
else
  log_fail "clippy -p ap"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  ap proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
