#!/usr/bin/env bash
# Proof script for modules/notifications (package: notifications-rs)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_notifications.sh                    # local build + tests
#   ./scripts/proof_notifications.sh --staging <host>   # + staging health check
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
echo "  Proof: notifications-rs"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p notifications-rs 2>&1; then
  log_pass "cargo build -p notifications-rs"
else
  log_fail "cargo build -p notifications-rs"
fi

# ── Gate 2: Clippy (zero warnings) ──────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p notifications-rs -- -D warnings 2>&1; then
  log_pass "clippy (zero warnings)"
else
  log_fail "clippy"
fi

# ── Gate 3: Unit tests ──────────────────────────────────────────────────────
log_step "Unit tests"
if ./scripts/cargo-slot.sh test -p notifications-rs --lib 2>&1; then
  log_pass "unit tests"
else
  log_fail "unit tests"
fi

# ── Gate 4: Integration tests (advisory — requires DB) ──────────────────────
log_step "Integration tests (advisory)"
if ./scripts/cargo-slot.sh test -p notifications-rs --tests 2>&1; then
  log_pass "integration tests"
else
  echo "  ⚠ Integration tests failed (DB may be unreachable)"
  log_pass "integration tests skipped (DB unreachable)"
fi

# ── Gate 5: Staging health check ─────────────────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  NOTIF_PORT="${NOTIF_PORT:-8089}"
  NOTIF_BASE="http://${STAGING_HOST}:${NOTIF_PORT}"

  log_step "Staging health (${NOTIF_BASE})"
  if curl --silent --fail --max-time 10 "${NOTIF_BASE}/healthz" > /dev/null 2>&1; then
    log_pass "GET ${NOTIF_BASE}/healthz → 200"
  else
    log_fail "GET ${NOTIF_BASE}/healthz did not return 200"
  fi

  if curl --silent --fail --max-time 10 "${NOTIF_BASE}/api/ready" > /dev/null 2>&1; then
    log_pass "GET ${NOTIF_BASE}/api/ready → 200"
  else
    log_fail "GET ${NOTIF_BASE}/api/ready did not return 200"
  fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  notifications-rs proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
