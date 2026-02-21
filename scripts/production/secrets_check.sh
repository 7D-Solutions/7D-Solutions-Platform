#!/usr/bin/env bash
# secrets_check.sh — Validate the production secrets file before deployment.
#
# Checks:
#   1. File exists at the expected path
#   2. File is owned by root (uid 0)
#   3. File has mode 0600 (no group/world read)
#   4. No CHANGE_ME placeholder values remain
#   5. No obvious test/development secrets (test_password, test_secret, etc.)
#   6. Required variables are present and non-empty
#
# Usage:
#   sudo bash scripts/production/secrets_check.sh [secrets-file]
#
# Default secrets file: /etc/7d/production/secrets.env
# Exit code 0 = all checks pass. Non-zero = one or more checks failed.

set -euo pipefail

SECRETS_FILE="${1:-/etc/7d/production/secrets.env}"
FAILURES=0

fail() {
    echo "FAIL: $*" >&2
    FAILURES=$((FAILURES + 1))
}

pass() {
    echo "PASS: $*"
}

echo "=== Production secrets check: $SECRETS_FILE ==="
echo ""

# --- Check 1: File exists ---
if [ ! -f "$SECRETS_FILE" ]; then
    fail "File not found: $SECRETS_FILE"
    echo ""
    echo "Create it as root:"
    echo "  sudo mkdir -p /etc/7d/production"
    echo "  sudo install -m 0600 -o root /dev/null /etc/7d/production/secrets.env"
    echo "  sudo nano /etc/7d/production/secrets.env   # populate from env.example"
    echo "See docs/DEPLOYMENT-PRODUCTION.md → Environment Contract."
    exit 1
else
    pass "File exists: $SECRETS_FILE"
fi

# --- Check 2: Owner is root ---
FILE_OWNER="$(stat -c '%u' "$SECRETS_FILE" 2>/dev/null || stat -f '%u' "$SECRETS_FILE")"
if [ "$FILE_OWNER" != "0" ]; then
    fail "Must be owned by root (uid 0), got uid $FILE_OWNER — fix: sudo chown root:root $SECRETS_FILE"
else
    pass "Owner is root (uid 0)"
fi

# --- Check 3: Mode is 0600 ---
FILE_MODE="$(stat -c '%a' "$SECRETS_FILE" 2>/dev/null || stat -f '%Lp' "$SECRETS_FILE")"
if [ "$FILE_MODE" != "600" ]; then
    fail "Mode must be 0600, got $FILE_MODE — fix: sudo chmod 0600 $SECRETS_FILE"
else
    pass "Mode is 0600"
fi

# --- Check 4: No CHANGE_ME placeholders ---
if grep -q "CHANGE_ME" "$SECRETS_FILE"; then
    PLACEHOLDER_LINES=$(grep -n "CHANGE_ME" "$SECRETS_FILE")
    fail "Contains CHANGE_ME placeholders (replace before deploying):"$'\n'"$PLACEHOLDER_LINES"
else
    pass "No CHANGE_ME placeholders"
fi

# --- Check 5: No obvious test/development secrets ---
TEST_PATTERNS=("test_password" "test_secret" "test_key" "dev_password" "dev_secret" \
               "localhost_password" "changeit" "password123" "secret123" "insecure")
FOUND_TEST=0
for pattern in "${TEST_PATTERNS[@]}"; do
    if grep -qi "$pattern" "$SECRETS_FILE"; then
        MATCHED=$(grep -in "$pattern" "$SECRETS_FILE")
        fail "Contains test/dev secret pattern '$pattern':"$'\n'"$MATCHED"
        FOUND_TEST=1
    fi
done
if [ "$FOUND_TEST" -eq 0 ]; then
    pass "No test/development secret patterns detected"
fi

# --- Check 6: Required variables present and non-empty ---
REQUIRED_VARS=(
    "AUTH_POSTGRES_PASSWORD"
    "AR_POSTGRES_PASSWORD"
    "SUBSCRIPTIONS_POSTGRES_PASSWORD"
    "PAYMENTS_POSTGRES_PASSWORD"
    "NOTIFICATIONS_POSTGRES_PASSWORD"
    "GL_POSTGRES_PASSWORD"
    "PROJECTIONS_POSTGRES_PASSWORD"
    "AUDIT_POSTGRES_PASSWORD"
    "TENANT_REGISTRY_POSTGRES_PASSWORD"
    "INVENTORY_POSTGRES_PASSWORD"
    "AP_POSTGRES_PASSWORD"
    "TREASURY_POSTGRES_PASSWORD"
    "FIXED_ASSETS_POSTGRES_PASSWORD"
    "CONSOLIDATION_POSTGRES_PASSWORD"
    "TIMEKEEPING_POSTGRES_PASSWORD"
    "PARTY_POSTGRES_PASSWORD"
    "INTEGRATIONS_POSTGRES_PASSWORD"
    "TTP_POSTGRES_PASSWORD"
    "JWT_PRIVATE_KEY_PEM"
    "JWT_PUBLIC_KEY_PEM"
    "JWT_SECRET"
)

MISSING=()
for var in "${REQUIRED_VARS[@]}"; do
    if ! grep -q "^${var}=" "$SECRETS_FILE"; then
        MISSING+=("$var")
    else
        VALUE=$(grep "^${var}=" "$SECRETS_FILE" | cut -d= -f2-)
        if [ -z "$VALUE" ]; then
            MISSING+=("${var} (empty)")
        fi
    fi
done

if [ ${#MISSING[@]} -gt 0 ]; then
    fail "Missing or empty required variables:"
    for v in "${MISSING[@]}"; do
        echo "  - $v" >&2
    done
else
    pass "All ${#REQUIRED_VARS[@]} required variables are present"
fi

# --- Summary ---
echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "✓ All checks passed — secrets file is valid for production deployment."
    exit 0
else
    echo "✗ $FAILURES check(s) failed — fix the issues above before deploying." >&2
    exit 1
fi
