#!/usr/bin/env bash
# tenantctl_verify.sh — Verify tenantctl CLI behavior
#
# Usage:
#   ./scripts/tenantctl_verify.sh --lifecycle    # Full lifecycle test (needs compose stack)
#   ./scripts/tenantctl_verify.sh --cli-only     # CLI parsing + help only (no services)
#
# Exit codes: 0 = all passed, 1 = failure

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PASS=0
FAIL=0

pass() {
    PASS=$((PASS + 1))
    echo "  ✓ $1"
}

fail() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
}

# ============================================================
# CLI parsing tests (no services needed)
# ============================================================

echo "=== tenantctl CLI verification ==="
echo ""
echo "--- CLI structure ---"

# Build first
echo "Building tenantctl..."
"$PROJECT_ROOT/scripts/cargo-slot.sh" build -p tenantctl 2>/dev/null

# Use the compiled binary directly for faster checks
BINARY=$(find "$PROJECT_ROOT"/target-slot-*/debug/tenantctl -maxdepth 0 -type f 2>/dev/null | head -1)
if [ -z "$BINARY" ]; then
    echo "ERROR: Cannot find compiled tenantctl binary"
    exit 1
fi
echo "Using binary: $BINARY"
echo ""

# Help text checks
if "$BINARY" --help >/dev/null 2>&1; then
    pass "tenantctl --help succeeds"
else
    fail "tenantctl --help succeeds"
fi

HELP_OUTPUT=$("$BINARY" --help 2>&1)
if echo "$HELP_OUTPUT" | grep -q 'tenant'; then
    pass "tenant subcommand in help"
else
    fail "tenant subcommand in help"
fi

if echo "$HELP_OUTPUT" | grep -q 'fleet'; then
    pass "fleet subcommand in help"
else
    fail "fleet subcommand in help"
fi

TENANT_HELP=$("$BINARY" tenant --help 2>&1)
if echo "$TENANT_HELP" | grep -q 'show'; then
    pass "tenant show command in help"
else
    fail "tenant show command in help"
fi

# --json flag
if echo "$HELP_OUTPUT" | grep -q '\-\-json'; then
    pass "--json flag in help"
else
    fail "--json flag in help"
fi

# Verify CLI structure via unit tests
echo ""
echo "--- Unit tests ---"
if "$PROJECT_ROOT/scripts/cargo-slot.sh" test -p tenantctl >/dev/null 2>&1; then
    pass "cargo test passes"
else
    fail "cargo test passes"
fi

# ============================================================
# Lifecycle tests (needs compose stack)
# ============================================================

if [ "${1:-}" = "--lifecycle" ]; then
    echo ""
    echo "--- Lifecycle tests (against local compose) ---"

    export TENANT_REGISTRY_DATABASE_URL="${TENANT_REGISTRY_DATABASE_URL:-postgres://platform_user:platform_pass@localhost:5433/tenant_registry}"
    export PLATFORM_AUDIT_DATABASE_URL="${PLATFORM_AUDIT_DATABASE_URL:-postgres://platform_user:platform_pass@localhost:5433/platform_audit}"

    TENANT_ID="verify-test-$(date +%s)"
    echo "  Using test tenant: $TENANT_ID"

    # tenant show nonexistent (should fail gracefully)
    OUTPUT=$("$BINARY" tenant show --tenant "$TENANT_ID" --json 2>&1 || true)
    if echo "$OUTPUT" | grep -q '"success"'; then
        pass "show --json returns structured output"
    else
        fail "show --json returns structured output"
    fi

    # JSON shape validation
    if echo "$OUTPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert 'success' in d
assert 'action' in d
assert 'tenant_id' in d
" 2>/dev/null; then
        pass "JSON output has required fields (success, action, tenant_id)"
    else
        fail "JSON output missing required fields"
    fi
fi

# ============================================================
# Summary
# ============================================================

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
