#!/bin/bash
# Generic dev entrypoint for 7D platform services.
# Requires SERVICE_BINARY env var to be set (path to the binary).
# Starts supervisord which manages the service process and binary watcher.

set -euo pipefail

if [ -z "${SERVICE_BINARY:-}" ]; then
  echo "[entrypoint] ERROR: SERVICE_BINARY env var must be set" >&2
  exit 1
fi

SUPERVISOR_CONF="${SUPERVISOR_CONF:-/etc/supervisor/conf.d/supervisord.conf}"

if [ ! -f "$SUPERVISOR_CONF" ]; then
  echo "[entrypoint] Missing supervisord config: $SUPERVISOR_CONF" >&2
  exit 1
fi

echo "[entrypoint] Starting service: $SERVICE_BINARY"
exec /usr/bin/supervisord -c "$SUPERVISOR_CONF"
