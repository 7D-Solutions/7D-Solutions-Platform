#!/usr/bin/env bash
# Proof script for modules/consolidation (package: consolidation)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_consolidation.sh
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
echo "  Proof: consolidation"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p consolidation 2>&1; then
  log_pass "cargo build -p consolidation"
else
  log_fail "cargo build -p consolidation"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p consolidation 2>&1; then
  log_pass "cargo test -p consolidation"
else
  log_fail "cargo test -p consolidation"
fi

# ── Gate 3: Clippy ───────────────────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p consolidation -- -D warnings 2>&1; then
  log_pass "clippy -p consolidation (zero warnings)"
else
  log_fail "clippy -p consolidation"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  consolidation proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
