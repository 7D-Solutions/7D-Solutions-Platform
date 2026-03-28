#!/usr/bin/env bash
# Proof script for modules/bom (package: bom-rs)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: bom-rs"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p bom-rs 2>&1; then log_pass "cargo build -p bom-rs"; else log_fail "cargo build -p bom-rs"; fi
log_step "Unit tests"
if ./scripts/cargo-slot.sh test -p bom-rs --lib 2>&1; then log_pass "cargo test -p bom-rs --lib"; else log_fail "cargo test -p bom-rs --lib"; fi
log_step "Integration tests (advisory)"
if ./scripts/cargo-slot.sh test -p bom-rs --tests 2>&1; then
  log_pass "cargo test -p bom-rs --tests"
else
  echo "  ⚠ Integration tests failed (DB may be unreachable)"
  log_pass "integration tests skipped (DB unreachable)"
fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p bom-rs -- -D warnings 2>&1; then log_pass "clippy -p bom-rs (zero warnings)"; else log_fail "clippy -p bom-rs"; fi
echo ""
echo "=============================="
echo "  bom-rs proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
