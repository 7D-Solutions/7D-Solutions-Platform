#!/usr/bin/env bash
# fix-tracking-files.sh - Detect and fix stale agent tracking files
#
# Problem: Multiple agents can have tracking files for the same bead,
# but only one is actually assigned in issues.jsonl
#
# Usage:
#   ./fix-tracking-files.sh --check     # Show problems
#   ./fix-tracking-files.sh --fix       # Fix stale tracking files

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

MODE="${1:---check}"

echo "=== Agent Tracking File Audit ==="
echo

# Get all tracking files
TRACKING_FILES=(/tmp/agent-bead-*.txt)
if [ ! -e "${TRACKING_FILES[0]}" ]; then
    echo "No tracking files found."
    exit 0
fi

# Map of bead_id -> list of agents claiming it
declare -A bead_to_agents
declare -A agent_to_bead

# Read all tracking files
for file in "${TRACKING_FILES[@]}"; do
    if [ -f "$file" ]; then
        agent_name=$(basename "$file" .txt | sed 's/^agent-bead-//')
        bead_id=$(cat "$file" 2>/dev/null || echo "")

        if [ -n "$bead_id" ]; then
            agent_to_bead["$agent_name"]="$bead_id"
            bead_to_agents["$bead_id"]+="$agent_name "
        fi
    fi
done

# Check each bead for conflicts
ISSUES_FILE="$PROJECT_DIR/.beads/issues.jsonl"
if [ ! -f "$ISSUES_FILE" ]; then
    echo "⚠️  No issues.jsonl found in $PROJECT_DIR"
    exit 1
fi

CONFLICTS=0
STALE_FILES=()

for bead_id in "${!bead_to_agents[@]}"; do
    agents_claiming="${bead_to_agents[$bead_id]}"
    agent_count=$(echo "$agents_claiming" | wc -w | tr -d ' ')

    # Get actual assignee from issues.jsonl
    actual_assignee=$(grep "\"id\":\"$bead_id\"" "$ISSUES_FILE" | tail -1 | jq -r '.assignee // empty' 2>/dev/null || echo "")

    if [ "$agent_count" -gt 1 ]; then
        echo "⚠️  CONFLICT: $bead_id claimed by $agent_count agents:"
        echo "   Tracking files: $agents_claiming"
        echo "   Actual assignee in issues.jsonl: ${actual_assignee:-<none>}"
        CONFLICTS=$((CONFLICTS + 1))

        # Mark all except actual assignee as stale
        for agent in $agents_claiming; do
            if [ "$agent" != "$actual_assignee" ]; then
                STALE_FILES+=("/tmp/agent-bead-$agent.txt")
                echo "   → /tmp/agent-bead-$agent.txt is STALE"
            fi
        done
        echo
    elif [ -n "$actual_assignee" ] && [ "$agents_claiming" != "$actual_assignee " ]; then
        # Single tracking file but wrong assignee
        echo "⚠️  MISMATCH: $bead_id"
        echo "   Tracking file: $agents_claiming"
        echo "   Actual assignee: $actual_assignee"
        STALE_FILES+=("/tmp/agent-bead-${agents_claiming% }.txt")
        CONFLICTS=$((CONFLICTS + 1))
        echo
    fi
done

if [ $CONFLICTS -eq 0 ]; then
    echo "✓ All tracking files match issues.jsonl"
    exit 0
fi

echo "Found $CONFLICTS conflicts, ${#STALE_FILES[@]} stale tracking files"
echo

if [ "$MODE" = "--fix" ]; then
    echo "=== Fixing stale tracking files ==="
    for file in "${STALE_FILES[@]}"; do
        if [ -f "$file" ]; then
            echo "Removing: $file"
            rm -f "$file"
        fi
    done
    echo "✓ Fixed ${#STALE_FILES[@]} stale tracking files"
else
    echo "Run with --fix to remove stale tracking files"
fi
