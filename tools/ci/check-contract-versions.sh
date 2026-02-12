#!/usr/bin/env bash
# Check that contract schema changes include version bumps

set -euo pipefail

echo "🔍 Checking contract version bumps..."

# Get changed schema files
changed_schemas=$(git diff --name-only origin/main...HEAD | grep 'contracts/events/.*\.json' || true)

if [ -z "$changed_schemas" ]; then
  echo "✓ No event schema changes detected"
  exit 0
fi

errors=()

for schema in $changed_schemas; do
  echo "  Checking $schema..."

  # Check if file was added (new schema)
  if ! git show origin/main:$schema &>/dev/null; then
    echo "    ✓ New schema file (no version check needed)"
    continue
  fi

  # Extract $id version from old and new schemas
  old_id=$(git show origin/main:$schema | jq -r '."$id"' 2>/dev/null || echo "")
  new_id=$(cat $schema | jq -r '."$id"' 2>/dev/null || echo "")

  if [ "$old_id" == "$new_id" ]; then
    errors+=("$schema: Schema changed but \$id version not bumped (was: $old_id)")
  else
    echo "    ✓ Version bumped: $old_id → $new_id"
  fi
done

if [ ${#errors[@]} -gt 0 ]; then
  echo ""
  echo "❌ ERROR: Schema version violations detected:" >&2
  printf '%s\n' "${errors[@]}" >&2
  echo "" >&2
  echo "When modifying schemas, you must bump the version in \$id field:" >&2
  echo "  PATCH: Bug fixes, clarifications" >&2
  echo "  MINOR: Add optional fields" >&2
  echo "  MAJOR: Breaking changes (new .vX file)" >&2
  exit 1
fi

echo "✓ All schema changes have proper version bumps"
