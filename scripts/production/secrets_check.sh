#!/usr/bin/env bash
# secrets_check.sh — Validate production secrets before deployment.
#
# Supports two formats:
#   --format=file  (default)  Single secrets.env file at /etc/7d/production/secrets.env
#   --format=dir              Directory of secret files at /etc/7d/production/secrets/
#
# Auto-detects format if not specified: uses dir if /etc/7d/production/secrets/ exists,
# otherwise falls back to file.
#
# Checks:
#   1. File/directory exists with correct ownership and permissions
#   2. No CHANGE_ME placeholder values
#   3. No obvious test/development secrets
#   4. All required secrets are present and non-empty
#
# Usage:
#   sudo bash scripts/production/secrets_check.sh
#   sudo bash scripts/production/secrets_check.sh --format=dir
#   sudo bash scripts/production/secrets_check.sh --format=file /path/to/secrets.env
#
# Exit code 0 = all checks pass. Non-zero = one or more checks failed.

set -euo pipefail

FORMAT=""
SECRETS_FILE="/etc/7d/production/secrets.env"
SECRETS_DIR="/etc/7d/production/secrets"
FAILURES=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --format=file) FORMAT="file"; shift ;;
        --format=dir)  FORMAT="dir";  shift ;;
        --format=*)    echo "ERROR: Unknown format. Use --format=file or --format=dir" >&2; exit 1 ;;
        *) SECRETS_FILE="$1"; shift ;;
    esac
done

# Auto-detect format
if [ -z "$FORMAT" ]; then
    if [ -d "$SECRETS_DIR" ]; then
        FORMAT="dir"
    else
        FORMAT="file"
    fi
fi

fail() {
    echo "FAIL: $*" >&2
    FAILURES=$((FAILURES + 1))
}

pass() {
    echo "PASS: $*"
}

# Test/development patterns that should never appear in production
TEST_PATTERNS=("test_password" "test_secret" "test_key" "dev_password" "dev_secret" \
               "localhost_password" "changeit" "password123" "secret123" "insecure" \
               "changeme123" "dev-nats-token" "dev-placeholder")

# Required secrets
REQUIRED_DB_PREFIXES=(
    "auth" "ar" "subscriptions" "payments" "notifications" "gl"
    "projections" "audit" "tenant_registry" "inventory" "ap" "treasury"
    "fixed_assets" "consolidation" "timekeeping" "party" "integrations"
    "ttp" "maintenance" "pdf_editor" "shipping_receiving" "numbering"
    "doc_mgmt" "workflow" "wc"
)

# =========================================================================
# FILE FORMAT CHECKS
# =========================================================================
check_file_format() {
    echo "=== Production secrets check (file format): $SECRETS_FILE ==="
    echo ""

    # Check 1: File exists
    if [ ! -f "$SECRETS_FILE" ]; then
        fail "File not found: $SECRETS_FILE"
        echo "  Create it: sudo install -m 0600 -o root /dev/null $SECRETS_FILE"
        exit 1
    else
        pass "File exists: $SECRETS_FILE"
    fi

    # Check 2: Owner is root
    FILE_OWNER="$(stat -c '%u' "$SECRETS_FILE" 2>/dev/null || stat -f '%u' "$SECRETS_FILE")"
    if [ "$FILE_OWNER" != "0" ]; then
        fail "Must be owned by root (uid 0), got uid $FILE_OWNER"
    else
        pass "Owner is root (uid 0)"
    fi

    # Check 3: Mode is 0600
    FILE_MODE="$(stat -c '%a' "$SECRETS_FILE" 2>/dev/null || stat -f '%Lp' "$SECRETS_FILE")"
    if [ "$FILE_MODE" != "600" ]; then
        fail "Mode must be 0600, got $FILE_MODE"
    else
        pass "Mode is 0600"
    fi

    # Check 4: No CHANGE_ME placeholders
    if grep -q "CHANGE_ME" "$SECRETS_FILE"; then
        fail "Contains CHANGE_ME placeholders"
    else
        pass "No CHANGE_ME placeholders"
    fi

    # Check 5: No test/dev patterns
    local found_test=0
    for pattern in "${TEST_PATTERNS[@]}"; do
        if grep -qi "$pattern" "$SECRETS_FILE"; then
            fail "Contains test/dev pattern: '$pattern'"
            found_test=1
        fi
    done
    if [ "$found_test" -eq 0 ]; then
        pass "No test/development patterns"
    fi

    # Check 6: Required variables
    REQUIRED_VARS=("JWT_PRIVATE_KEY_PEM" "JWT_PUBLIC_KEY_PEM" "JWT_SECRET" "NATS_AUTH_TOKEN" "SEED_ADMIN_PASSWORD")
    for prefix in "${REQUIRED_DB_PREFIXES[@]}"; do
        REQUIRED_VARS+=("$(echo "${prefix}" | tr '[:lower:]' '[:upper:]')_POSTGRES_PASSWORD")
    done

    local missing=0
    for var in "${REQUIRED_VARS[@]}"; do
        if ! grep -q "^${var}=" "$SECRETS_FILE"; then
            fail "Missing: $var"
            missing=$((missing + 1))
        fi
    done
    if [ "$missing" -eq 0 ]; then
        pass "All ${#REQUIRED_VARS[@]} required variables present"
    fi
}

# =========================================================================
# DIRECTORY FORMAT CHECKS
# =========================================================================
check_dir_format() {
    echo "=== Production secrets check (directory format): $SECRETS_DIR ==="
    echo ""

    # Check 1: Directory exists
    if [ ! -d "$SECRETS_DIR" ]; then
        fail "Directory not found: $SECRETS_DIR"
        echo "  Run: sudo bash scripts/production/secrets_init.sh"
        exit 1
    else
        pass "Directory exists: $SECRETS_DIR"
    fi

    # Check 2: Directory ownership and mode
    DIR_OWNER="$(stat -c '%u' "$SECRETS_DIR" 2>/dev/null || stat -f '%u' "$SECRETS_DIR")"
    if [ "$DIR_OWNER" != "0" ]; then
        fail "Directory must be owned by root (uid 0), got uid $DIR_OWNER"
    else
        pass "Directory owner is root"
    fi

    DIR_MODE="$(stat -c '%a' "$SECRETS_DIR" 2>/dev/null || stat -f '%Lp' "$SECRETS_DIR")"
    if [ "$DIR_MODE" != "700" ]; then
        fail "Directory mode must be 0700, got $DIR_MODE"
    else
        pass "Directory mode is 0700"
    fi

    # Check 3: All files are 0600 and root-owned
    local bad_perms=0
    for f in "$SECRETS_DIR"/*; do
        [ -f "$f" ] || continue
        local f_owner f_mode
        f_owner="$(stat -c '%u' "$f" 2>/dev/null || stat -f '%u' "$f")"
        f_mode="$(stat -c '%a' "$f" 2>/dev/null || stat -f '%Lp' "$f")"
        if [ "$f_owner" != "0" ] || [ "$f_mode" != "600" ]; then
            fail "Bad permissions on $(basename "$f"): owner=$f_owner mode=$f_mode (want root/0600)"
            bad_perms=$((bad_perms + 1))
        fi
    done
    if [ "$bad_perms" -eq 0 ]; then
        pass "All files are root:root 0600"
    fi

    # Check 4: No CHANGE_ME / test patterns in any file
    local found_bad=0
    for f in "$SECRETS_DIR"/*; do
        [ -f "$f" ] || continue
        local content
        content="$(cat "$f")"
        if echo "$content" | grep -q "CHANGE_ME"; then
            fail "$(basename "$f") contains CHANGE_ME placeholder"
            found_bad=1
        fi
        for pattern in "${TEST_PATTERNS[@]}"; do
            if echo "$content" | grep -qi "$pattern"; then
                fail "$(basename "$f") contains test/dev pattern: '$pattern'"
                found_bad=1
            fi
        done
    done
    if [ "$found_bad" -eq 0 ]; then
        pass "No placeholder or test/dev values"
    fi

    # Check 5: Required secret files exist and are non-empty
    REQUIRED_FILES=(
        "jwt_private_key_pem" "jwt_public_key_pem" "jwt_secret"
        "nats_auth_token" "nats_url"
        "seed_admin_password" "tilled_webhook_secret"
        "control_plane_ar_database_url"
    )
    for prefix in "${REQUIRED_DB_PREFIXES[@]}"; do
        REQUIRED_FILES+=("${prefix}_postgres_password" "${prefix}_database_url")
    done

    local missing=0
    for name in "${REQUIRED_FILES[@]}"; do
        local path="${SECRETS_DIR}/${name}"
        if [ ! -f "$path" ]; then
            fail "Missing secret file: $name"
            missing=$((missing + 1))
        elif [ ! -s "$path" ]; then
            fail "Empty secret file: $name"
            missing=$((missing + 1))
        fi
    done
    if [ "$missing" -eq 0 ]; then
        pass "All ${#REQUIRED_FILES[@]} required secret files present"
    fi

    # Check 6: DATABASE_URL secrets contain valid postgres:// prefix
    local bad_urls=0
    for prefix in "${REQUIRED_DB_PREFIXES[@]}"; do
        local url_file="${SECRETS_DIR}/${prefix}_database_url"
        if [ -f "$url_file" ]; then
            local url_content
            url_content="$(cat "$url_file")"
            if [[ "$url_content" != postgres://* ]]; then
                fail "${prefix}_database_url does not start with postgres://"
                bad_urls=$((bad_urls + 1))
            fi
        fi
    done
    if [ "$bad_urls" -eq 0 ]; then
        pass "All DATABASE_URL secrets have valid postgres:// prefix"
    fi
}

# =========================================================================
# Main
# =========================================================================
if [ "$FORMAT" = "dir" ]; then
    check_dir_format
else
    check_file_format
fi

echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "All checks passed — secrets are valid for production deployment."
    exit 0
else
    echo "$FAILURES check(s) failed — fix the issues above before deploying." >&2
    exit 1
fi
