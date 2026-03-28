#!/usr/bin/env bash
# Proof script for tools/tenantctl (package: tenantctl)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: tenantctl"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p tenantctl 2>&1; then log_pass "cargo build -p tenantctl"; else log_fail "cargo build -p tenantctl"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p tenantctl 2>&1; then log_pass "cargo test -p tenantctl"; else log_fail "cargo test -p tenantctl"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p tenantctl -- -D warnings 2>&1; then log_pass "clippy -p tenantctl (zero warnings)"; else log_fail "clippy -p tenantctl"; fi
echo ""
echo "=============================="
echo "  tenantctl proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
