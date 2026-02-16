#!/usr/bin/env bash
set -euo pipefail

# Add dependencies to Phase 16 beads

JSON_FILE="tmp/phase16-beads.json"
MAPPING_FILE="tmp/phase16-id-mapping.txt"

if [[ ! -f "$JSON_FILE" ]] || [[ ! -f "$MAPPING_FILE" ]]; then
  echo "Error: Required files not found"
  exit 1
fi

TOTAL=$(jq 'length' "$JSON_FILE")

echo "========================================"
echo "Adding dependencies to Phase 16 beads"
echo "========================================"
echo

for i in $(seq 0 $(($TOTAL - 1))); do
  BEAD=$(jq -r ".[$i]" "$JSON_FILE")

  CHATGPT_ID=$(echo "$BEAD" | jq -r '.id')
  DEPS=$(echo "$BEAD" | jq -r '.depends_on[]' 2>/dev/null || echo "")

  if [[ -n "$DEPS" ]]; then
    # Get actual ID from mapping
    ACTUAL_ID=$(grep "^$CHATGPT_ID=" "$MAPPING_FILE" | cut -d'=' -f2)

    echo "[$((i+1))/$TOTAL] $ACTUAL_ID ($CHATGPT_ID):"

    for dep_chatgpt_id in $DEPS; do
      dep_actual_id=$(grep "^$dep_chatgpt_id=" "$MAPPING_FILE" | cut -d'=' -f2)
      echo "  → depends on $dep_actual_id ($dep_chatgpt_id)"

      # Add dependency: ACTUAL_ID depends on dep_actual_id
      br dep add "$ACTUAL_ID" "$dep_actual_id" --type blocks 2>&1 | grep -v "Auto-flush" || {
        echo "    ✗ Failed to add dependency"
        exit 1
      }
    done
    echo "  ✓ Dependencies added"
  fi
done

echo
echo "========================================"
echo "✅ All dependencies added successfully"
echo "========================================"
