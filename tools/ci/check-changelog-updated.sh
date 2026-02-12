#!/usr/bin/env bash
# Check that CHANGELOG.md is updated when contracts change

set -euo pipefail

echo "🔍 Checking CHANGELOG update..."

# Get changed contract files
contract_changes=$(git diff --name-only origin/main...HEAD | grep '^contracts/' || true)

if [ -z "$contract_changes" ]; then
  echo "✓ No contract changes detected"
  exit 0
fi

# Check if CHANGELOG.md was updated
if ! git diff --name-only origin/main...HEAD | grep -q 'CHANGELOG.md'; then
  echo ""
  echo "❌ ERROR: Contract files changed but CHANGELOG.md not updated" >&2
  echo "" >&2
  echo "Changed contract files:" >&2
  echo "$contract_changes" >&2
  echo "" >&2
  echo "Update CHANGELOG.md with:" >&2
  echo "  - Version number" >&2
  echo "  - Date" >&2
  echo "  - Brief description of changes" >&2
  echo "  - Deprecation notices (if applicable)" >&2
  echo "" >&2
  echo "See docs/architecture/CONTRACT-VERSIONING-POLICY.md" >&2
  exit 1
fi

echo "✓ CHANGELOG.md updated"
