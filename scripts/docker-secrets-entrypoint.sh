#!/bin/sh
# docker-secrets-entrypoint.sh — Bridge Docker secrets to environment variables.
#
# Reads each file in /run/secrets/ and exports it as an environment variable
# named after the file (uppercase). Then exec's the original command.
#
# Secret file naming convention:
#   /run/secrets/DATABASE_URL  → exports DATABASE_URL=<file contents>
#   /run/secrets/NATS_URL      → exports NATS_URL=<file contents>
#
# Multi-line secrets (e.g. PEM keys) are preserved as-is.
# If a secret file exists, it OVERRIDES any env var with the same name.
# If no secret files exist, the original env vars pass through unchanged
# (allows the same image to work in dev without secrets).
#
# Usage in docker-compose.production.yml:
#   entrypoint: ["/usr/local/bin/secrets-entrypoint.sh"]
#   command: ["identity-auth"]

set -e

SECRETS_DIR="/run/secrets"

if [ -d "$SECRETS_DIR" ]; then
    for secret_file in "$SECRETS_DIR"/*; do
        [ -f "$secret_file" ] || continue
        var_name="$(basename "$secret_file")"
        # Skip hidden files
        case "$var_name" in .*) continue ;; esac
        export "$var_name"="$(cat "$secret_file")"
    done
fi

exec "$@"
