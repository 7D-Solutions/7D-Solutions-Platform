#!/usr/bin/env bash
# Proof script for tools/contract-tests (package: contract-tests)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: contract-tests"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p contract-tests 2>&1; then log_pass "cargo build -p contract-tests"; else log_fail "cargo build -p contract-tests"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p contract-tests 2>&1; then log_pass "cargo test -p contract-tests"; else log_fail "cargo test -p contract-tests"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p contract-tests -- -D warnings 2>&1; then log_pass "clippy -p contract-tests (zero warnings)"; else log_fail "clippy -p contract-tests"; fi
echo ""
echo "=============================="
echo "  contract-tests proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
