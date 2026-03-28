#!/usr/bin/env bash
# Proof script for tools/stabilization-gate (package: stabilization-gate)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
PASS=0; FAIL=0
log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
echo "=============================="
echo "  Proof: stabilization-gate"
echo "=============================="
log_step "Build"
if ./scripts/cargo-slot.sh build -p stabilization-gate 2>&1; then log_pass "cargo build -p stabilization-gate"; else log_fail "cargo build -p stabilization-gate"; fi
log_step "Tests"
if ./scripts/cargo-slot.sh test -p stabilization-gate 2>&1; then log_pass "cargo test -p stabilization-gate"; else log_fail "cargo test -p stabilization-gate"; fi
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p stabilization-gate -- -D warnings 2>&1; then log_pass "clippy -p stabilization-gate (zero warnings)"; else log_fail "clippy -p stabilization-gate"; fi
echo ""
echo "=============================="
echo "  stabilization-gate proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then echo "PROOF FAILED — do not promote."; exit 1; fi
echo "PROOF PASSED — safe to promote."
