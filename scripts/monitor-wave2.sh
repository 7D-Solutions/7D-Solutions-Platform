#!/usr/bin/env bash
# Monitor Wave 2 plug-and-play progress
# Run: ./scripts/monitor-wave2.sh

PROJECT_ROOT="/Users/james/Projects/7D-Solutions Platform"
cd "$PROJECT_ROOT"

while true; do
  OPEN=$(br list -s open 2>&1 | wc -l | tr -d ' ')
  IN_PROG=$(br list -s in_progress 2>&1 | wc -l | tr -d ' ')
  CLOSED_TODAY=$(br list -s closed 2>&1 | grep "2026-03-30\|Plug-and-play\|Split:" | wc -l | tr -d ' ')
  READY=$(br ready 2>&1 | grep -c "bd-" || echo 0)
  
  TIMESTAMP=$(date +%H:%M:%S)
  
  echo "[$TIMESTAMP] Open: $OPEN | In Progress: $IN_PROG | Ready to claim: $READY"
  
  # Show who's working on what
  IN_PROG_LIST=$(br list -s in_progress 2>&1 | head -5)
  if [ -n "$IN_PROG_LIST" ]; then
    echo "$IN_PROG_LIST" | while read line; do
      echo "  → $line"
    done
  fi
  
  # Check for compile errors
  BUILD_OK=$(./scripts/cargo-slot.sh build --workspace 2>&1 | tail -1)
  if echo "$BUILD_OK" | grep -q "Finished"; then
    echo "  Build: OK"
  else
    echo "  Build: FAILING — $BUILD_OK"
  fi
  
  echo "---"
  
  # Exit when all done
  if [ "$OPEN" -eq 0 ] && [ "$IN_PROG" -eq 0 ]; then
    echo "ALL BEADS COMPLETE"
    break
  fi
  
  sleep 300  # Check every 5 minutes
done
