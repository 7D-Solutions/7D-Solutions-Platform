#!/usr/bin/env bash
# export_env.sh — Load a staging env file and export all variables.
#
# Usage:
#   source scripts/staging/export_env.sh [env-file]
#
# Default env file: scripts/staging/.env.staging
# If sourced, variables are exported into the current shell.
# If executed directly, prints the export commands (useful for subshells).
#
# Example:
#   source scripts/staging/export_env.sh
#   source scripts/staging/export_env.sh /path/to/custom.env

set -euo pipefail

ENV_FILE="${1:-$(dirname "$0")/.env.staging}"

if [ ! -f "$ENV_FILE" ]; then
    echo "ERROR: env file not found: $ENV_FILE" >&2
    echo "Copy scripts/staging/env.example to scripts/staging/.env.staging and populate it." >&2
    exit 1
fi

# Validate no CHANGE_ME placeholders remain
if grep -q "CHANGE_ME" "$ENV_FILE"; then
    echo "ERROR: $ENV_FILE contains CHANGE_ME placeholders." >&2
    echo "Replace all CHANGE_ME values before deploying." >&2
    grep -n "CHANGE_ME" "$ENV_FILE" >&2
    exit 1
fi

# Export each non-comment, non-empty line
while IFS= read -r line; do
    # Skip blank lines and comments
    [[ -z "$line" || "$line" == \#* ]] && continue
    # Validate the line is an assignment
    if [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
        export "$line"
    fi
done < "$ENV_FILE"

echo "✓ Exported environment from $ENV_FILE" >&2
