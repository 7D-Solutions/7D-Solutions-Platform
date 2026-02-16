#!/bin/bash
# Stop browser worker

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$PROJECT_ROOT"

if [ ! -f .flywheel/browser-worker-pid.txt ]; then
  echo "Worker not running (no PID file)"
  exit 0
fi

WORKER_PID=$(cat .flywheel/browser-worker-pid.txt)

if ! kill -0 "$WORKER_PID" 2>/dev/null; then
  echo "Worker not running (stale PID: $WORKER_PID)"
  rm -f .flywheel/browser-worker-pid.txt .flywheel/browser-ready.txt
  exit 0
fi

echo "Stopping worker (PID: $WORKER_PID)..."
kill -TERM "$WORKER_PID" 2>/dev/null || true

# Wait for graceful shutdown (max 5 seconds)
for i in {1..10}; do
  if ! kill -0 "$WORKER_PID" 2>/dev/null; then
    echo "✓ Worker stopped gracefully"
    rm -f .flywheel/browser-worker-pid.txt .flywheel/browser-ready.txt
    exit 0
  fi
  sleep 0.5
done

# Force kill if still running
if kill -0 "$WORKER_PID" 2>/dev/null; then
  echo "Worker didn't stop gracefully, forcing..."
  kill -9 "$WORKER_PID" 2>/dev/null || true
  echo "✓ Worker force killed"
fi

rm -f .flywheel/browser-worker-pid.txt .flywheel/browser-ready.txt
