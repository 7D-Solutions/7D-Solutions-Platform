#!/usr/bin/env bash
set -euo pipefail
echo "Simulating DB outage (docker compose multi assumed)."
docker compose -f docker-compose.multi.yml stop postgres || true
echo "DB stopped. /health/ready should fail and sensitive flows should block."
echo ""
echo "Restart with:"
echo "  docker compose -f docker-compose.multi.yml start postgres"
