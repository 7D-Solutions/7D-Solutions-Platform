#!/bin/bash
# Start browser worker if not already running

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$PROJECT_ROOT"

# Check if already running
if [ -f .flywheel/browser-worker-pid.txt ]; then
  WORKER_PID=$(cat .flywheel/browser-worker-pid.txt)
  if kill -0 "$WORKER_PID" 2>/dev/null; then
    echo "✓ Worker already running (PID: $WORKER_PID)"
    exit 0
  else
    echo "Cleaning stale PID file"
    rm -f .flywheel/browser-worker-pid.txt .flywheel/browser-ready.txt
  fi
fi

echo "Starting browser worker..."

# Start worker
node scripts/chatgpt/browser-worker.mjs > .flywheel/browser-worker.log 2>&1 &
WORKER_PID=$!
echo $WORKER_PID > .flywheel/browser-worker-pid.txt

echo "Worker PID: $WORKER_PID"

# Wait for ready signal (max 10 seconds)
echo -n "Waiting for worker to be ready..."
for i in {1..20}; do
  if [ -f .flywheel/browser-ready.txt ]; then
    echo " ✓"
    echo "✓ Worker started successfully"
    exit 0
  fi
  echo -n "."
  sleep 0.5
done

echo " ✗"
echo "ERROR: Worker failed to start within 10 seconds"
echo ""
echo "Last 20 lines of worker log:"
tail -20 .flywheel/browser-worker.log
exit 1
