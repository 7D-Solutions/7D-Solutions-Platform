#!/usr/bin/env bash
# Proof script for platform/health (package: health)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_health.sh
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
echo "  Proof: health"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p health 2>&1; then
  log_pass "cargo build -p health"
else
  log_fail "cargo build -p health"
fi

# ── Gate 2: Clippy (zero warnings) ──────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p health -- -D warnings 2>&1; then
  log_pass "cargo clippy -p health -- -D warnings"
else
  log_fail "cargo clippy -p health -- -D warnings"
fi

# ── Gate 3: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p health 2>&1; then
  log_pass "cargo test -p health"
else
  log_fail "cargo test -p health"
fi

# ── Gate 4: Public API surface check ─────────────────────────────────────────
log_step "Public API surface"

# Verify key types and functions are exported
EXPECTED_EXPORTS=(
  "pub enum CheckStatus"
  "pub enum ReadyStatus"
  "pub struct HealthCheck"
  "pub struct ReadyResponse"
  "pub struct HealthzResponse"
  "pub struct PoolMetrics"
  "pub async fn healthz"
  "pub fn build_ready_response"
  "pub fn ready_response_to_axum"
  "pub fn db_check"
  "pub fn db_check_with_pool"
  "pub fn nats_check"
)

api_ok=true
for export in "${EXPECTED_EXPORTS[@]}"; do
  if ! grep -q "$export" platform/health/src/lib.rs; then
    log_fail "Missing export: $export"
    api_ok=false
  fi
done

if $api_ok; then
  log_pass "All expected public API items present"
fi

# ── Gate 5: SQL injection guard (no raw SQL in this crate) ────────────────────
log_step "SQL injection guard"
if grep -rn 'format!.*SELECT\|format!.*INSERT\|format!.*UPDATE\|format!.*DELETE' platform/health/src/ 2>/dev/null; then
  log_fail "Found raw SQL string formatting — potential injection risk"
else
  log_pass "No raw SQL string formatting found"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  health proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
