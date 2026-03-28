#!/usr/bin/env bash
# Proof script for modules/integrations (package: integrations-rs)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_integrations.sh                    # local build + unit tests only
#   ./scripts/proof_integrations.sh --staging <host>   # + staging health checks
#
# Environment variables (override CLI flags):
#   STAGING_HOST              — VPS hostname or IP
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST="${STAGING_HOST:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staging) STAGING_HOST="$2"; shift 2 ;;
    *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

PASS=0
FAIL=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

echo "=============================="
echo "  Proof: integrations (integrations-rs)"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p integrations-rs 2>&1; then
  log_pass "cargo build -p integrations-rs"
else
  log_fail "cargo build -p integrations-rs"
fi

# ── Gate 2: Clippy ───────────────────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p integrations-rs -- -D warnings 2>&1; then
  log_pass "cargo clippy -p integrations-rs"
else
  log_fail "cargo clippy -p integrations-rs"
fi

# ── Gate 3: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p integrations-rs 2>&1; then
  log_pass "cargo test -p integrations-rs"
else
  log_fail "cargo test -p integrations-rs"
fi

# ── Gate 4: Staging health check ─────────────────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  INTEGRATIONS_PORT="${INTEGRATIONS_PORT:-8099}"
  INTEGRATIONS_BASE="http://${STAGING_HOST}:${INTEGRATIONS_PORT}"

  log_step "Staging health (${INTEGRATIONS_BASE})"
  if curl --silent --fail --max-time 10 "${INTEGRATIONS_BASE}/healthz" > /dev/null 2>&1; then
    log_pass "GET ${INTEGRATIONS_BASE}/healthz → 200"
  else
    log_fail "GET ${INTEGRATIONS_BASE}/healthz did not return 200"
  fi

  if curl --silent --fail --max-time 10 "${INTEGRATIONS_BASE}/api/ready" > /dev/null 2>&1; then
    log_pass "GET ${INTEGRATIONS_BASE}/api/ready → 200"
  else
    log_fail "GET ${INTEGRATIONS_BASE}/api/ready did not return 200"
  fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  integrations proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
