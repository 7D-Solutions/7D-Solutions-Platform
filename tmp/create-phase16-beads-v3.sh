#!/usr/bin/env bash
set -euo pipefail

# Create all Phase 16 beads from JSON spec
# Phase 1: Create all beads and build ID mapping (using temp file instead of associative array)
# Phase 2: Add dependencies using actual IDs

JSON_FILE="tmp/phase16-beads.json"
MAPPING_FILE="tmp/phase16-id-mapping.txt"
MAPPING_JSON="tmp/phase16-id-mapping.json"

if [[ ! -f "$JSON_FILE" ]]; then
  echo "Error: $JSON_FILE not found"
  exit 1
fi

# Count beads
TOTAL=$(jq 'length' "$JSON_FILE")
echo "========================================"
echo "Phase 16 Bead Creation (27 beads)"
echo "========================================"
echo

# Phase 1: Create beads without dependencies
echo "PHASE 1: Creating beads..."
echo

> "$MAPPING_FILE"  # Clear mapping file

for i in $(seq 0 $(($TOTAL - 1))); do
  BEAD=$(jq -r ".[$i]" "$JSON_FILE")

  CHATGPT_ID=$(echo "$BEAD" | jq -r '.id')
  CODE=$(echo "$BEAD" | jq -r '.code')
  TITLE=$(echo "$BEAD" | jq -r '.title')
  PRIORITY=$(echo "$BEAD" | jq -r '.priority')

  echo "[$(($i+1))/$TOTAL] $CHATGPT_ID ($CODE): $TITLE"

  # Create bead
  ACTUAL_ID=$(br create \
    --title "$TITLE" \
    --priority "$PRIORITY" \
    --type task \
    --labels "phase16,$CODE" \
    --description "ChatGPT-ID: $CHATGPT_ID | Code: $CODE" \
    --silent)

  if [[ -n "$ACTUAL_ID" ]]; then
    echo "  ✓ $ACTUAL_ID"
    echo "$CHATGPT_ID=$ACTUAL_ID" >> "$MAPPING_FILE"
  else
    echo "  ✗ Failed"
    exit 1
  fi
done

echo
echo "✓ All beads created"
echo

# Convert mapping to JSON for easier reading
echo "{" > "$MAPPING_JSON"
while IFS='=' read -r chatgpt_id actual_id; do
  echo "  \"$chatgpt_id\": \"$actual_id\"," >> "$MAPPING_JSON"
done < "$MAPPING_FILE"
sed -i.bak '$ s/,$//' "$MAPPING_JSON" && rm -f "$MAPPING_JSON.bak"
echo "}" >> "$MAPPING_JSON"

echo "Mapping: $MAPPING_JSON"
cat "$MAPPING_JSON"
echo

# Phase 2: Add dependencies
echo "========================================"
echo "PHASE 2: Adding dependencies..."
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

      # Add dependency (this bead blocks the dependency)
      br update "$ACTUAL_ID" --add-dep "blocks:$dep_actual_id" 2>&1 | grep -v "Auto-flush" || true
    done
  fi
done

echo
echo "========================================"
echo "✅ SUCCESS: 27 Phase 16 beads created"
echo "========================================"
echo
echo "Verify: br list --labels phase16"
echo "Mapping: cat $MAPPING_JSON"
