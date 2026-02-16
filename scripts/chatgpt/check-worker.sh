#!/bin/bash
# Check browser worker status

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$PROJECT_ROOT"

echo "=== Browser Worker Status ==="
echo ""

# Check PID file
if [ ! -f .flywheel/browser-worker-pid.txt ]; then
  echo "Status: NOT RUNNING (no PID file)"
  exit 1
fi

WORKER_PID=$(cat .flywheel/browser-worker-pid.txt)

# Check if process is alive
if ! kill -0 "$WORKER_PID" 2>/dev/null; then
  echo "Status: NOT RUNNING (stale PID: $WORKER_PID)"
  exit 1
fi

echo "Status: RUNNING"
echo "PID: $WORKER_PID"
echo ""

# Check ready file
if [ -f .flywheel/browser-ready.txt ]; then
  READY_TIME=$(cat .flywheel/browser-ready.txt)
  echo "Ready since: $(date -r $(($READY_TIME / 1000)) 2>/dev/null || echo 'unknown')"
else
  echo "Ready: NO (missing ready file)"
fi

echo ""

# Check recent activity
if [ -f .flywheel/browser-worker.log ]; then
  echo "Recent activity:"
  tail -5 .flywheel/browser-worker.log | sed 's/^/  /'
  echo ""

  # Check for recent errors
  if tail -10 .flywheel/browser-worker.log | grep -q "ERROR\|✗"; then
    echo "⚠️  Recent errors detected in log"
  else
    echo "✓ No recent errors"
  fi
fi

exit 0
