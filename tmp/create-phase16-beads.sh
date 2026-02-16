#!/bin/bash
set -euo pipefail

# Create all Phase 16 beads from JSON spec

JSON_FILE="tmp/phase16-beads.json"

if [[ ! -f "$JSON_FILE" ]]; then
  echo "Error: $JSON_FILE not found"
  exit 1
fi

# Count beads
TOTAL=$(jq 'length' "$JSON_FILE")
echo "Creating $TOTAL Phase 16 beads..."
echo

CREATED=0
FAILED=0

# Iterate through each bead
for i in $(seq 0 $(($TOTAL - 1))); do
  BEAD=$(jq -r ".[$i]" "$JSON_FILE")

  ID=$(echo "$BEAD" | jq -r '.id')
  TITLE=$(echo "$BEAD" | jq -r '.title')
  PRIORITY=$(echo "$BEAD" | jq -r '.priority')
  DEPENDS_ON=$(echo "$BEAD" | jq -r '.depends_on | join(",")')

  echo "[$((i+1))/$TOTAL] Creating $ID: $TITLE"

  # Create bead
  if br create "$ID" \
      --title "$TITLE" \
      --priority "$PRIORITY" \
      --type task \
      ${DEPENDS_ON:+--deps "$DEPENDS_ON"} 2>&1 | tee /tmp/bead-create-$ID.log; then
    ((CREATED++))
    echo "  ✓ Created"
  else
    ((FAILED++))
    echo "  ✗ Failed"
  fi
  echo
done

echo "================================================"
echo "Summary: $CREATED created, $FAILED failed"
echo "================================================"

if [[ $FAILED -gt 0 ]]; then
  exit 1
fi
