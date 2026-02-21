#!/usr/bin/env bash
# new_revision_entry.sh — Generate a REVISIONS.md entry stub for a target module/version.
#
# Usage:
#   bash scripts/versioning/new_revision_entry.sh <module-path> <version> [bead-id]
#
# Arguments:
#   module-path   Relative path to the module directory (e.g., modules/ar, platform/identity-auth)
#   version       SemVer version being added (e.g., 1.0.0, 1.1.0)
#   bead-id       Optional bead ID (defaults to bd-xxxx placeholder)
#
# Examples:
#   bash scripts/versioning/new_revision_entry.sh modules/ar 1.0.0 bd-qvbg
#   bash scripts/versioning/new_revision_entry.sh platform/identity-auth 1.2.0 bd-yyyy
#
# Behavior:
#   - If REVISIONS.md does not exist in the module directory, creates it from the
#     canonical template (docs/templates/MODULE-REVISIONS.md) with the new row.
#   - If REVISIONS.md already exists, inserts a new row after the last row in the
#     ## Revisions table. Does not duplicate an existing row for the same version.
#
# Required fields (see also lint_revisions.sh):
#   Version          — SemVer from package file
#   Date             — ISO date (YYYY-MM-DD) of the commit
#   Bead             — Bead ID tracking this work (e.g., bd-xxxx)
#   What Changed     — Concrete description of the change (not TODO or generic)
#   Why              — Reason the change was necessary
#   Breaking?        — "No" or "YES: <migration note>"
#
# Proof command requirement:
#   Before promoting a module to 1.0.0, a proof script must exist at:
#     scripts/proof_{module_basename}.sh  (or scripts/proof_{module_basename_underscored}.sh)
#   This is enforced by Gate 1 (pre-commit hook) and documented in docs/VERSIONING.md.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

usage() {
    echo "Usage: $0 <module-path> <version> [bead-id]" >&2
    echo "" >&2
    echo "Examples:" >&2
    echo "  $0 modules/ar 1.0.0 bd-qvbg" >&2
    echo "  $0 platform/tenant-registry 1.0.0 bd-tzsh" >&2
    echo "  $0 platform/identity-auth 1.2.0 bd-yyyy" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------

MODULE_PATH="${1:-}"
VERSION="${2:-}"
BEAD_ID="${3:-bd-xxxx}"

if [[ -z "$MODULE_PATH" || -z "$VERSION" ]]; then
    usage
fi

# Validate version format
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "ERROR: Version must be SemVer format (e.g., 1.0.0): '$VERSION'" >&2
    exit 1
fi

MODULE_ABS="$REPO_ROOT/$MODULE_PATH"
if [[ ! -d "$MODULE_ABS" ]]; then
    echo "ERROR: Module directory not found: $MODULE_PATH" >&2
    exit 1
fi

MODULE_NAME=$(basename "$MODULE_PATH")
TODAY=$(date +%Y-%m-%d)
REVISIONS_FILE="$MODULE_ABS/REVISIONS.md"

# ---------------------------------------------------------------------------
# Build the new row content
# ---------------------------------------------------------------------------

MAJOR=$(echo "$VERSION" | cut -d. -f1)

if [[ "$VERSION" == "1.0.0" ]]; then
    WHAT_CHANGED="Initial proof. TODO: list each capability proven (endpoints, events, behaviors)."
    WHY="Module build complete. All E2E tests passing."
    BREAKING="—"
else
    WHAT_CHANGED="TODO: describe what changed — name specific endpoints, fields, events, or behaviors."
    WHY="TODO: describe why this change was necessary."
    if [[ "$MAJOR" -ge 2 ]]; then
        BREAKING="YES: TODO — describe what consumers must change and the migration path."
    else
        BREAKING="No"
    fi
fi

TABLE_ROW="| $VERSION | $TODAY | $BEAD_ID | $WHAT_CHANGED | $WHY | $BREAKING |"

# ---------------------------------------------------------------------------
# Derive proof script name for reminder
# ---------------------------------------------------------------------------

# Convert hyphens to underscores for script name
PROOF_SCRIPT_NAME="proof_${MODULE_NAME//-/_}.sh"
PROOF_SCRIPT_PATH="$REPO_ROOT/scripts/$PROOF_SCRIPT_NAME"

# ---------------------------------------------------------------------------
# Create or update REVISIONS.md
# ---------------------------------------------------------------------------

if [[ ! -f "$REVISIONS_FILE" ]]; then
    # Create new REVISIONS.md
    cat > "$REVISIONS_FILE" <<REVEOF
# ${MODULE_NAME} — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See \`docs/VERSIONING.md\` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | \`scripts/${PROOF_SCRIPT_NAME}\` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
${TABLE_ROW}

## How to read this table

- **Version:** The version in the package file (\`Cargo.toml\` or \`package.json\`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** \`No\` if existing consumers are unaffected. \`YES\` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
REVEOF

    echo "✓ Created: $REVISIONS_FILE"
    echo "  Row added for version $VERSION"
else
    # REVISIONS.md already exists — check for duplicate then insert row
    if grep -qE "^\| $VERSION " "$REVISIONS_FILE"; then
        echo "WARNING: REVISIONS.md already has an entry for version $VERSION" >&2
        echo "  File: $REVISIONS_FILE" >&2
        echo "  Skipping insertion. Update the existing row manually if needed." >&2
        exit 0
    fi

    # Insert new row after the last table row in the ## Revisions section
    python3 - "$REVISIONS_FILE" "$TABLE_ROW" <<'PYEOF'
import sys

filepath = sys.argv[1]
new_row = sys.argv[2]

with open(filepath) as f:
    lines = f.readlines()

in_revisions = False
last_table_row_idx = -1

for i, line in enumerate(lines):
    stripped = line.rstrip('\n')
    if stripped.startswith('## Revisions'):
        in_revisions = True
        continue
    if stripped.startswith('## ') and in_revisions:
        in_revisions = False
    if in_revisions and stripped.startswith('|') and not stripped.startswith('|---'):
        last_table_row_idx = i

if last_table_row_idx == -1:
    print("ERROR: Could not find Revisions table in REVISIONS.md", file=sys.stderr)
    sys.exit(1)

lines.insert(last_table_row_idx + 1, new_row + '\n')

with open(filepath, 'w') as f:
    f.writelines(lines)
PYEOF

    echo "✓ Updated: $REVISIONS_FILE"
    echo "  Row inserted for version $VERSION"
fi

# ---------------------------------------------------------------------------
# Proof script reminder for 1.0.0 promotions
# ---------------------------------------------------------------------------

if [[ "$VERSION" == "1.0.0" ]]; then
    echo ""
    if [[ -f "$PROOF_SCRIPT_PATH" ]]; then
        echo "✓ Proof script exists: scripts/$PROOF_SCRIPT_NAME"
    else
        echo "REMINDER: Proof script required before committing 1.0.0."
        echo "  Expected: scripts/$PROOF_SCRIPT_NAME"
        echo "  Create it or Gate 1 will reject the commit."
    fi
fi

echo ""
echo "Next: fill in the TODO placeholders in $REVISIONS_FILE"
echo "      then commit with: git commit -m \"[$BEAD_ID] $MODULE_NAME v$VERSION: <description>\""
