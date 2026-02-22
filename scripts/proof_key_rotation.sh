#!/usr/bin/env bash
# Proof gate: zero-downtime JWT and webhook secret rotation rehearsal.
#
# Validates that the platform's overlap mechanism works correctly:
#   - JWT: old-key-signed tokens accepted during overlap, rejected after
#   - Webhook: old-secret-signed payloads accepted during overlap, rejected after
#   - JWKS: both keys served during overlap
#
# Usage:
#   ./scripts/proof_key_rotation.sh
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

echo "============================================="
echo "  Proof: Key Rotation Rehearsal (bd-26ro)"
echo "============================================="

# ── Gate 1: Build identity-auth ───────────────────────────────────────────────
log_step "Build identity-auth"
if ./scripts/cargo-slot.sh build -p auth-rs 2>&1; then
    log_pass "cargo build -p auth-rs"
else
    log_fail "cargo build -p auth-rs"
fi

# ── Gate 2: JWT rotation overlap tests (auth-rs) ──────────────────────────────
log_step "JWT rotation overlap tests (auth-rs::auth::jwt)"
if ./scripts/cargo-slot.sh test -p auth-rs -- auth::jwt::tests 2>&1; then
    log_pass "JWT round-trip, expiry, and rotation overlap tests"
else
    log_fail "JWT tests failed — check auth-rs::auth::jwt::tests"
fi

# ── Gate 3: Build platform/security ───────────────────────────────────────────
log_step "Build platform/security"
if ./scripts/cargo-slot.sh build -p security 2>&1; then
    log_pass "cargo build -p security"
else
    log_fail "cargo build -p security"
fi

# ── Gate 4: JWT verifier overlap tests (security crate) ───────────────────────
log_step "JWT verifier rotation overlap tests (security::claims)"
if ./scripts/cargo-slot.sh test -p security -- claims::tests 2>&1; then
    log_pass "JwtVerifier rotation overlap: old-key accepted during window, rejected after"
else
    log_fail "security claims tests failed"
fi

# ── Gate 5: Webhook signature rotation overlap tests ──────────────────────────
log_step "Webhook signature rotation overlap tests (payments-rs::webhook_signature)"
# Use --lib to test unit tests only (integration tests require a live DB)
if ./scripts/cargo-slot.sh test -p payments-rs --lib -- webhook_signature 2>&1; then
    log_pass "Webhook rotation overlap: old-secret accepted during window, rejected after"
else
    log_fail "payments-rs webhook_signature tests failed"
fi

# ── Gate 6: Full unit test suites for auth-rs and security ────────────────────
log_step "Full unit test suites: auth-rs, security"
for pkg in auth-rs security; do
    if ./scripts/cargo-slot.sh test -p "$pkg" 2>&1; then
        log_pass "cargo test -p $pkg"
    else
        log_fail "cargo test -p $pkg"
    fi
done

# payments-rs unit tests (--lib only; integration tests require live DB)
log_step "payments-rs unit tests (--lib)"
if ./scripts/cargo-slot.sh test -p payments-rs --lib 2>&1; then
    log_pass "cargo test -p payments-rs --lib"
else
    log_fail "cargo test -p payments-rs --lib"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "============================================="
echo "  Key rotation proof: ${PASS} pass / ${FAIL} fail"
echo "============================================="
if [[ $FAIL -gt 0 ]]; then
    echo "PROOF FAILED — rotation rehearsal not complete."
    exit 1
fi
echo "PROOF PASSED — zero-downtime rotation rehearsal verified."
echo ""
echo "Next steps for live rotation:"
echo "  1. Generate new RSA key pair (see docs/runbooks/key_rotation.md §1)"
echo "  2. Set JWT_PREV_PUBLIC_KEY_PEM + JWT_PREV_KID on identity-auth, rolling restart"
echo "  3. Set JWT_PUBLIC_KEY_PREV on all module services, rolling restart"
echo "  4. Wait one token TTL (15 min default)"
echo "  5. Clear *_PREV vars, rolling restart"
echo "  6. Verify JWKS serves only new key: curl <auth>/. well-known/jwks.json | jq '.keys|length'"
