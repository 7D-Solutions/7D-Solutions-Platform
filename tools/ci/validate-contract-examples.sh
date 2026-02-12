#!/usr/bin/env bash
# Validate that all contract schemas have valid examples

set -euo pipefail

echo "🔍 Validating contract examples..."

errors=()

for schema in contracts/events/*.json; do
  [ ! -f "$schema" ] && continue

  echo "  Checking $schema..."

  # Check if schema has examples field
  if ! jq -e '.examples' "$schema" > /dev/null 2>&1; then
    errors+=("$schema: Missing 'examples' field")
    continue
  fi

  # Check if examples is a non-empty array
  example_count=$(jq -r '.examples | length' "$schema" 2>/dev/null || echo "0")
  if [ "$example_count" -eq 0 ]; then
    errors+=("$schema: 'examples' array is empty")
    continue
  fi

  # Validate each example has required envelope fields
  for i in $(seq 0 $((example_count - 1))); do
    example=$(jq -r ".examples[$i]" "$schema")

    for field in event_id occurred_at tenant_id source_module source_version payload; do
      if ! echo "$example" | jq -e ".$field" > /dev/null 2>&1; then
        errors+=("$schema: Example $i missing required field '$field'")
      fi
    done
  done

  echo "    ✓ $example_count example(s) present and valid"
done

if [ ${#errors[@]} -gt 0 ]; then
  echo ""
  echo "❌ ERROR: Contract example violations detected:" >&2
  printf '%s\n' "${errors[@]}" >&2
  echo "" >&2
  echo "All schemas must include at least one valid example." >&2
  echo "See docs/architecture/CONTRACT-VERSIONING-POLICY.md" >&2
  exit 1
fi

echo "✓ All contracts have valid examples"
