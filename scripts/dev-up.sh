#!/usr/bin/env bash
# Bring up the local dev stack with automatic orphan cleanup.
#
# Usage:
#   scripts/dev-up.sh              # Start data + services
#   scripts/dev-up.sh data         # Start data stack only (Postgres, NATS)
#   scripts/dev-up.sh services     # Start services only (assumes data is running)
#   scripts/dev-up.sh down         # Stop everything
#
# This always passes --remove-orphans to prevent ghost containers from
# accumulating after container recreations.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

COMPOSE_DATA="docker-compose.data.yml"
COMPOSE_MAIN="docker-compose.yml"  # includes docker-compose.services.yml

case "${1:-all}" in
  data)
    echo "Starting data stack (Postgres, NATS)..."
    docker compose -f "$COMPOSE_DATA" up -d --remove-orphans
    ;;
  services)
    echo "Starting services..."
    docker compose -f "$COMPOSE_MAIN" up -d --remove-orphans
    ;;
  down)
    echo "Stopping all stacks..."
    docker compose -f "$COMPOSE_MAIN" down --remove-orphans
    docker compose -f "$COMPOSE_DATA" down --remove-orphans
    ;;
  all)
    echo "Starting data stack..."
    docker compose -f "$COMPOSE_DATA" up -d --remove-orphans
    echo ""
    echo "Starting services..."
    docker compose -f "$COMPOSE_MAIN" up -d --remove-orphans
    ;;
  *)
    echo "Usage: scripts/dev-up.sh [data|services|down]"
    exit 1
    ;;
esac

echo ""
echo "Done. Running containers:"
docker compose -f "$COMPOSE_DATA" -f "$COMPOSE_MAIN" ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}" 2>/dev/null | head -30
