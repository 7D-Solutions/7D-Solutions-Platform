#!/bin/bash
set -euo pipefail

# Create all Phase 16 beads from JSON spec
# Phase 1: Create all beads without dependencies and build ID mapping
# Phase 2: Add dependencies using actual IDs

JSON_FILE="tmp/phase16-beads.json"
MAPPING_FILE="tmp/phase16-id-mapping.json"

if [[ ! -f "$JSON_FILE" ]]; then
  echo "Error: $JSON_FILE not found"
  exit 1
fi

# Count beads
TOTAL=$(jq 'length' "$JSON_FILE")
echo "================================================"
echo "Phase 16 Bead Creation (27 beads)"
echo "================================================"
echo

# Phase 1: Create beads and build mapping
echo "PHASE 1: Creating beads without dependencies..."
echo

declare -A ID_MAP

for i in $(seq 0 $(($TOTAL - 1))); do
  BEAD=$(jq -r ".[$i]" "$JSON_FILE")

  CHATGPT_ID=$(echo "$BEAD" | jq -r '.id')
  CODE=$(echo "$BEAD" | jq -r '.code')
  TITLE=$(echo "$BEAD" | jq -r '.title')
  PRIORITY=$(echo "$BEAD" | jq -r '.priority')

  echo "[$(($i+1))/$TOTAL] Creating $CHATGPT_ID ($CODE): $TITLE"

  # Create bead with code as label for tracking
  ACTUAL_ID=$(br create \
    --title "$TITLE" \
    --priority "$PRIORITY" \
    --type task \
    --labels "phase16,$CODE" \
    --description "ChatGPT ID: $CHATGPT_ID" \
    --silent)

  if [[ -n "$ACTUAL_ID" ]]; then
    echo "  ✓ Created $ACTUAL_ID (ChatGPT: $CHATGPT_ID)"
    ID_MAP["$CHATGPT_ID"]="$ACTUAL_ID"
  else
    echo "  ✗ Failed to create $CHATGPT_ID"
    exit 1
  fi
done

echo
echo "PHASE 1 COMPLETE: All beads created"
echo

# Save mapping to file
echo "{" > "$MAPPING_FILE"
for chatgpt_id in "${!ID_MAP[@]}"; do
  actual_id="${ID_MAP[$chatgpt_id]}"
  echo "  \"$chatgpt_id\": \"$actual_id\"," >> "$MAPPING_FILE"
done
# Remove trailing comma and close JSON
sed -i.bak '$ s/,$//' "$MAPPING_FILE" && rm "$MAPPING_FILE.bak"
echo "}" >> "$MAPPING_FILE"

echo "ID Mapping saved to: $MAPPING_FILE"
echo
cat "$MAPPING_FILE"
echo

# Phase 2: Add dependencies
echo "================================================"
echo "PHASE 2: Adding dependencies..."
echo "================================================"
echo

for i in $(seq 0 $(($TOTAL - 1))); do
  BEAD=$(jq -r ".[$i]" "$JSON_FILE")

  CHATGPT_ID=$(echo "$BEAD" | jq -r '.id')
  DEPENDS_ON=$(echo "$BEAD" | jq -r '.depends_on[]' 2>/dev/null || true)

  if [[ -n "$DEPENDS_ON" ]]; then
    ACTUAL_ID="${ID_MAP[$CHATGPT_ID]}"
    echo "[$((i+1))/$TOTAL] Adding dependencies for $ACTUAL_ID ($CHATGPT_ID):"

    for dep_chatgpt_id in $DEPENDS_ON; do
      dep_actual_id="${ID_MAP[$dep_chatgpt_id]}"
      echo "  → depends on $dep_actual_id ($dep_chatgpt_id)"

      # Add dependency
      br update "$ACTUAL_ID" --add-dep "blocks:$dep_actual_id" || {
        echo "    ✗ Failed to add dependency"
      }
    done
  fi
done

echo
echo "================================================"
echo "SUCCESS: All 27 Phase 16 beads created"
echo "================================================"
echo
echo "Next steps:"
echo "1. Verify beads: br list --labels phase16"
echo "2. View mapping: cat $MAPPING_FILE"
echo "3. Release to pool when ready"
