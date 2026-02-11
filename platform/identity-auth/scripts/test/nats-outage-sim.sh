#!/usr/bin/env bash
set -euo pipefail
echo "Simulating NATS outage (docker compose multi assumed)."
docker compose -f docker-compose.multi.yml stop nats || true
echo "NATS stopped. Login should still work (best-effort publish)."
echo ""
echo "Restart with:"
echo "  docker compose -f docker-compose.multi.yml start nats"
