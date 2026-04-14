#!/usr/bin/env bash
# up.sh — Bring up the local developer environment and wait for readiness.
#
# Usage:
#   ./scripts/dev/up.sh
#
# The script:
#   1. Verifies local prerequisites via scripts/dev/doctor.sh
#   2. Starts the data stack
#   3. Starts the application stack
#   4. Waits for all known services to report ready
#   5. Prints a green "ready" when the environment is usable

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

READY_TIMEOUT="${READY_TIMEOUT:-900}"
READY_INTERVAL="${READY_INTERVAL:-5}"

"$SCRIPT_DIR/doctor.sh"

echo "starting data stack..."
docker compose -f docker-compose.data.yml up -d

echo "starting application stack..."
docker compose up -d

READY_URLS=(
  "http://localhost:8080"
  "http://localhost:8091"
  "http://localhost:8086"
  "http://localhost:8087"
  "http://localhost:8088"
  "http://localhost:8089"
  "http://localhost:8090"
  "http://localhost:8092"
  "http://localhost:8093"
  "http://localhost:8094"
  "http://localhost:8096"
  "http://localhost:8097"
  "http://localhost:8098"
  "http://localhost:8099"
  "http://localhost:8100"
  "http://localhost:8101"
  "http://localhost:8102"
  "http://localhost:8103"
  "http://localhost:8104"
  "http://localhost:8105"
  "http://localhost:8106"
  "http://localhost:8107"
  "http://localhost:8108"
  "http://localhost:8111"
  "http://localhost:8120"
  "http://localhost:8121"
)

echo "waiting for service readiness..."
"$PROJECT_ROOT/scripts/dev/wait-for-ready.sh" \
  --timeout "$READY_TIMEOUT" \
  --interval "$READY_INTERVAL" \
  "${READY_URLS[@]}"

"$PROJECT_ROOT/scripts/verify_health_endpoints.sh"

printf '\033[32mready\033[0m\n'
