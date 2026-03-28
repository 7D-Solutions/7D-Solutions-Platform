#!/usr/bin/env bash
# Proof script for modules/gl (package: gl-rs)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_gl.sh                          # local build + unit tests only
#   ./scripts/proof_gl.sh --staging <host>         # + staging health check
#
# Environment variables (override CLI flags):
#   STAGING_HOST              — VPS hostname or IP
#   GL_PORT                   — GL service port (default: 8088)
#
# Exits 0 only when all checks pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

STAGING_HOST="${STAGING_HOST:-}"
GL_PORT="${GL_PORT:-8088}"

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
echo "  Proof: gl (gl-rs)"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p gl-rs 2>&1; then
  log_pass "cargo build -p gl-rs"
else
  log_fail "cargo build -p gl-rs"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests"
if ./scripts/cargo-slot.sh test -p gl-rs 2>&1; then
  log_pass "cargo test -p gl-rs"
else
  log_fail "cargo test -p gl-rs"
fi

# ── Gate 3: Clippy (lint check) ──────────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p gl-rs -- -D warnings 2>&1; then
  log_pass "cargo clippy -p gl-rs"
else
  log_fail "cargo clippy -p gl-rs"
fi

# ── Gate 4: Staging health check ─────────────────────────────────────────────
if [[ -n "$STAGING_HOST" ]]; then
  GL_BASE="http://${STAGING_HOST}:${GL_PORT}"

  log_step "Staging health (${GL_BASE})"
  if curl --silent --fail --max-time 10 "${GL_BASE}/healthz" > /dev/null 2>&1; then
    log_pass "GET ${GL_BASE}/healthz → 200"
  else
    log_fail "GET ${GL_BASE}/healthz did not return 200"
  fi

  if curl --silent --fail --max-time 10 "${GL_BASE}/api/ready" > /dev/null 2>&1; then
    log_pass "GET ${GL_BASE}/api/ready → 200"
  else
    log_fail "GET ${GL_BASE}/api/ready did not return 200"
  fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  gl proof: ${PASS} pass / ${FAIL} fail"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
