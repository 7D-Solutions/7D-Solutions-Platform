#!/usr/bin/env bash
# Proof script for tools/demo-seed (package: demo-seed)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: demo-seed"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p demo-seed 2>&1; then log_pass "cargo build -p demo-seed"; else log_fail "cargo build -p demo-seed"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p demo-seed 2>&1; then log_pass "cargo test -p demo-seed"; else log_fail "cargo test -p demo-seed"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p demo-seed -- -D warnings 2>&1; then log_pass "clippy -p demo-seed (zero warnings)"; else log_fail "clippy -p demo-seed"; fi
echo ""
echo "=============================="
echo "  demo-seed proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
