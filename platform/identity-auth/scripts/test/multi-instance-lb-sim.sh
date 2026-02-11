#!/usr/bin/env bash
set -euo pipefail
echo "Starting multi-instance stack..."
docker compose -f docker-compose.multi.yml up -d --build

echo ""
echo "Checking readiness..."
sleep 5
curl -fsS http://localhost:8080/health/ready >/dev/null && echo "✅ READY ok" || echo "❌ READY failed"

echo ""
echo "JWKS:"
curl -fsS http://localhost:8080/.well-known/jwks.json | head
