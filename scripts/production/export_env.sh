#!/usr/bin/env bash
# export_env.sh — Load the production secrets file and export all variables.
#
# Usage (on the production VPS, run as deploy user):
#   source scripts/production/export_env.sh [secrets-file]
#
# Default secrets file: /etc/7d/production/secrets.env
# The file must be root-owned (uid 0) and mode 0600.
#
# If sourced, variables are exported into the current shell.
# If executed directly, prints the export commands (useful for subshells).
#
# Example:
#   source scripts/production/export_env.sh
#   source scripts/production/export_env.sh /path/to/custom.env

set -euo pipefail

SECRETS_FILE="${1:-/etc/7d/production/secrets.env}"

# File must exist
if [ ! -f "$SECRETS_FILE" ]; then
    echo "ERROR: secrets file not found: $SECRETS_FILE" >&2
    echo "Create /etc/7d/production/secrets.env as root:" >&2
    echo "  sudo install -m 0600 -o root /dev/null /etc/7d/production/secrets.env" >&2
    echo "  sudo nano /etc/7d/production/secrets.env   # populate from env.example" >&2
    echo "See docs/DEPLOYMENT-PRODUCTION.md → Environment Contract." >&2
    exit 1
fi

# File must be owned by root (uid 0)
FILE_OWNER="$(stat -c '%u' "$SECRETS_FILE" 2>/dev/null || stat -f '%u' "$SECRETS_FILE")"
if [ "$FILE_OWNER" != "0" ]; then
    echo "ERROR: $SECRETS_FILE must be owned by root (uid 0), got uid $FILE_OWNER" >&2
    echo "Fix: sudo chown root:root $SECRETS_FILE" >&2
    exit 1
fi

# File must have mode 0600 (no group/world read)
FILE_MODE="$(stat -c '%a' "$SECRETS_FILE" 2>/dev/null || stat -f '%Lp' "$SECRETS_FILE")"
if [ "$FILE_MODE" != "600" ]; then
    echo "ERROR: $SECRETS_FILE must have mode 0600, got $FILE_MODE" >&2
    echo "Fix: sudo chmod 0600 $SECRETS_FILE" >&2
    exit 1
fi

# Validate no CHANGE_ME placeholders remain
if grep -q "CHANGE_ME" "$SECRETS_FILE"; then
    echo "ERROR: $SECRETS_FILE contains CHANGE_ME placeholders." >&2
    echo "Replace all CHANGE_ME values before deploying to production." >&2
    grep -n "CHANGE_ME" "$SECRETS_FILE" >&2
    exit 1
fi

# Export each non-comment, non-empty line
while IFS= read -r line; do
    # Skip blank lines and comments
    [[ -z "$line" || "$line" == \#* ]] && continue
    # Validate the line is a valid assignment
    if [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
        export "$line"
    fi
done < "$SECRETS_FILE"

echo "✓ Exported production secrets from $SECRETS_FILE" >&2
