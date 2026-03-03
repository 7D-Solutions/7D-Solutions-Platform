#!/usr/bin/env bash
# Start Docker Compose Watch for all services.
# Watches source directories and auto-rebuilds containers on changes.
#
# Usage:
#   scripts/dev-watch.sh              # Watch all services (via docker-compose.yml)
#   scripts/dev-watch.sh --legacy     # Watch using legacy split files
#
# Prerequisites:
#   - Docker Compose v2.22+ (compose watch support)
#   - Data stack running: docker compose -f docker-compose.data.yml up -d
#
# What gets watched:
#   - Each service watches its own src/ directory (targeted rebuild)
#   - All services watch Cargo.lock (dependency changes)
#   - All services watch platform/health/src and platform/security/src (shared crates)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Check docker compose version supports watch
COMPOSE_VERSION=$(docker compose version --short 2>/dev/null || echo "0.0.0")
MAJOR=$(echo "$COMPOSE_VERSION" | cut -d. -f1)
MINOR=$(echo "$COMPOSE_VERSION" | cut -d. -f2)
if [ "$MAJOR" -lt 2 ] || { [ "$MAJOR" -eq 2 ] && [ "$MINOR" -lt 22 ]; }; then
    echo "Error: Docker Compose v2.22+ required for watch support (found $COMPOSE_VERSION)" >&2
    exit 1
fi

if [ "${1:-}" = "--legacy" ]; then
    echo "Starting watch on legacy compose files..."
    echo "  - docker-compose.modules.yml"
    echo "  - docker-compose.platform.yml"
    echo ""
    echo "Press Ctrl+C to stop."
    echo ""
    docker compose \
        -f docker-compose.modules.yml \
        -f docker-compose.platform.yml \
        watch
else
    echo "Starting watch on all services..."
    echo "  - docker-compose.yml (includes docker-compose.services.yml)"
    echo ""
    echo "Press Ctrl+C to stop."
    echo ""
    docker compose watch
fi
