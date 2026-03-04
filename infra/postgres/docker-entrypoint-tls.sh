#!/usr/bin/env sh
# docker-entrypoint-tls.sh — Wrapper entrypoint for Postgres containers that
# copies TLS key to a location with correct permissions (600, owned by postgres)
# then delegates to the standard docker-entrypoint.sh.
#
# Postgres refuses to start if server.key is readable by group/other.
# Docker bind-mounts come in as root-owned, so we copy to the data dir.

set -e

TLS_SRC_DIR="/etc/postgresql/tls"
TLS_DST_DIR="/var/lib/postgresql/tls"

if [ -f "$TLS_SRC_DIR/server.crt" ] && [ -f "$TLS_SRC_DIR/server.key" ]; then
  mkdir -p "$TLS_DST_DIR"
  cp "$TLS_SRC_DIR/server.crt" "$TLS_DST_DIR/server.crt"
  cp "$TLS_SRC_DIR/server.key" "$TLS_DST_DIR/server.key"
  if [ -f "$TLS_SRC_DIR/ca.crt" ]; then
    cp "$TLS_SRC_DIR/ca.crt" "$TLS_DST_DIR/ca.crt"
  fi
  chmod 600 "$TLS_DST_DIR/server.key"
  chmod 644 "$TLS_DST_DIR/server.crt"
  chown -R postgres:postgres "$TLS_DST_DIR"
fi

# Remove stale PID if present (some containers need this after unclean shutdown)
rm -f /var/lib/postgresql/data/postmaster.pid

exec docker-entrypoint.sh "$@"
