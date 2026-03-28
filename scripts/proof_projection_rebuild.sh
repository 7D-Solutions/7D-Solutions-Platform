#!/usr/bin/env bash
# Proof script for tools/projection-rebuild (package: projection-rebuild)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: projection-rebuild"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p projection-rebuild 2>&1; then log_pass "cargo build -p projection-rebuild"; else log_fail "cargo build -p projection-rebuild"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p projection-rebuild 2>&1; then log_pass "cargo test -p projection-rebuild"; else log_fail "cargo test -p projection-rebuild"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p projection-rebuild -- -D warnings 2>&1; then log_pass "clippy -p projection-rebuild (zero warnings)"; else log_fail "clippy -p projection-rebuild"; fi
echo ""
echo "=============================="
echo "  projection-rebuild proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
