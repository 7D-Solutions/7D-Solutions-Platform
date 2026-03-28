#!/usr/bin/env bash
# Proof script for platform/event-bus (package: event-bus)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_event_bus.sh
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
echo "  Proof: event-bus"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p event-bus 2>&1; then
  log_pass "cargo build -p event-bus"
else
  log_fail "cargo build -p event-bus"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p event-bus 2>&1; then
  log_pass "cargo test -p event-bus"
else
  log_fail "cargo test -p event-bus"
fi

# ── Gate 3: Clippy (zero warnings) ──────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p event-bus -- -D warnings 2>&1; then
  log_pass "cargo clippy -p event-bus -- -D warnings"
else
  log_fail "cargo clippy -p event-bus -- -D warnings"
fi

# ── Gate 4: Doc tests ───────────────────────────────────────────────────────
log_step "Doc tests"
if ./scripts/cargo-slot.sh test -p event-bus --doc 2>&1; then
  log_pass "cargo test -p event-bus --doc"
else
  log_fail "cargo test -p event-bus --doc"
fi

# ── Gate 5: Downstream crates compile ───────────────────────────────────────
log_step "Downstream: event-consumer"
if ./scripts/cargo-slot.sh check -p event-consumer 2>&1; then
  log_pass "cargo check -p event-consumer"
else
  log_fail "cargo check -p event-consumer"
fi

log_step "Downstream: platform_contracts"
if ./scripts/cargo-slot.sh check -p platform_contracts 2>&1; then
  log_pass "cargo check -p platform_contracts"
else
  log_fail "cargo check -p platform_contracts"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  event-bus proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
