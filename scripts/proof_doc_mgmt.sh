#!/usr/bin/env bash
# Proof script for platform/doc-mgmt (package: doc_mgmt)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: doc_mgmt"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p doc_mgmt 2>&1; then log_pass "cargo build"; else log_fail "cargo build"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p doc_mgmt 2>&1; then log_pass "cargo test"; else log_fail "cargo test"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p doc_mgmt -- -D warnings 2>&1; then log_pass "clippy (zero warnings)"; else log_fail "clippy"; fi
echo ""
echo "=============================="
echo "  doc_mgmt proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
