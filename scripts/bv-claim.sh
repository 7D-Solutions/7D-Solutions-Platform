#!/bin/bash
# bv-claim: Get BV recommendation and claim task in one step
#
# Usage: bv-claim [--json|--toon] [--priority P0,P1,P2]
#
# Options:
#   --json          Output in JSON format (default)
#   --toon          Output in TOON format
#   --priority      Filter by priority (default: P0,P1,P2)
#
# Example: bv-claim
# Example: bv-claim --toon
# Example: bv-claim --priority P0,P1

set -euo pipefail

# Default values
FORMAT="json"
PRIORITY="P0,P1,P2"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --json)
            FORMAT="json"
            shift
            ;;
        --toon)
            FORMAT="toon"
            shift
            ;;
        --priority)
            PRIORITY="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: bv-claim [--json|--toon] [--priority P0,P1,P2]"
            echo ""
            echo "Get BV recommendation and claim the task automatically."
            echo ""
            echo "Options:"
            echo "  --json          Output in JSON format (default)"
            echo "  --toon          Output in TOON format"
            echo "  --priority      Filter by priority (default: P0,P1,P2)"
            exit 0
            ;;
        *)
            echo "Error: Unknown option $1"
            exit 1
            ;;
    esac
done

# Ensure BV is installed
if ! command -v bv >/dev/null 2>&1; then
    echo "Error: BV (Beads Viewer) is not installed."
    echo "Install with: curl -fsSL \"https://raw.githubusercontent.com/Dicklesworthstone/beads_viewer/main/install.sh\" | bash"
    exit 1
fi

# Ensure br is installed
if ! command -v br >/dev/null 2>&1; then
    echo "Error: br (beads) is not installed."
    echo "Install beads_rust first."
    exit 1
fi

# Ensure jq is installed for JSON parsing
if ! command -v jq >/dev/null 2>&1; then
    echo "Error: jq is required but not installed."
    echo "Install with: brew install jq (macOS) or apt-get install jq (Linux)"
    exit 1
fi

# Get agent name for ownership
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
OS_USER=$(whoami)
if [ "$AGENT_NAME" = "unknown" ] || [ "$AGENT_NAME" = "$OS_USER" ]; then
    echo "❌ Cannot claim beads without a registered agent identity." >&2
    echo "   Your agent name resolved to '$AGENT_NAME' which is not valid." >&2
    echo "   Register first: ./scripts/agent-mail-helper.sh register \"Your role\"" >&2
    exit 1
fi

# Sync beads data to JSONL format for BV
echo "🔄 Syncing beads data to JSONL format..."
br sync --flush-only --force 2>/dev/null || {
    echo "Warning: Could not sync beads data. BV may show outdated information."
}

# Get BV recommendation
echo "🤖 Getting BV recommendation..."
BV_OUTPUT=$(bv --robot-next --format "$FORMAT")

if [ "$FORMAT" = "json" ]; then
    # Parse JSON output
    TASK_ID=$(echo "$BV_OUTPUT" | jq -r '.id // empty')
    TASK_TITLE=$(echo "$BV_OUTPUT" | jq -r '.title // empty')
    SCORE=$(echo "$BV_OUTPUT" | jq -r '.score // empty')
    REASONS=$(echo "$BV_OUTPUT" | jq -r '.reasons[]? // empty' | head -3 | paste -sd ';' -)

    if [ -z "$TASK_ID" ] || [ "$TASK_ID" = "null" ]; then
        echo "⚠️  No actionable tasks found."
        echo "$BV_OUTPUT"
        exit 0
    fi

    # Check if task is already in progress
    IN_PROGRESS=$(echo "$BV_OUTPUT" | jq -r '.reasons[]? // empty' | grep -i "in progress" || true)

    echo "🎯 BV recommends: $TASK_ID - $TASK_TITLE"
    echo "   Score: $SCORE"
    if [ -n "$REASONS" ]; then
        echo "   Reasons: $REASONS"
    fi

    # Check if already in progress - skip claiming
    if [ -n "$IN_PROGRESS" ]; then
        echo ""
        echo "⚠️  Task $TASK_ID is already in progress - skipping claim."
        echo "   Use 'bd show $TASK_ID' to see details."
        exit 0
    fi

    # Claim the task (set owner and status)
    echo "📝 Claiming task $TASK_ID..."
    if br update "$TASK_ID" --status in_progress --owner "$AGENT_NAME" --assignee "$AGENT_NAME"; then
        echo "✅ Successfully claimed $TASK_ID (assignee: $AGENT_NAME)"

        # Show updated recommendation
        echo ""
        echo "📋 Next recommendation after claim:"
        bv --robot-next --format "$FORMAT"
    else
        echo "❌ Failed to claim $TASK_ID"
        exit 1
    fi

else
    # TOON format output
    echo "$BV_OUTPUT"

    # TOON format: bd-1z3|P2|Phase 4 decision review (after 4 weeks)
    TASK_ID=$(echo "$BV_OUTPUT" | cut -d'|' -f1)

    if [[ "$TASK_ID" =~ ^bd- ]]; then
        # For TOON, check status via br show
        TASK_STATUS=$(br show "$TASK_ID" --json 2>/dev/null | jq -r '.status // empty')

        if [ "$TASK_STATUS" = "in_progress" ]; then
            echo ""
            echo "⚠️  Task $TASK_ID is already in progress - skipping claim."
            echo "   Use 'bd show $TASK_ID' to see details."
            exit 0
        fi

        echo "📝 Claiming task $TASK_ID..."
        if br update "$TASK_ID" --status in_progress --owner "$AGENT_NAME" --assignee "$AGENT_NAME"; then
            echo "✅ Successfully claimed $TASK_ID (assignee: $AGENT_NAME)"

            # Show updated recommendation
            echo ""
            echo "📋 Next recommendation after claim:"
            bv --robot-next --format "$FORMAT"
        else
            echo "❌ Failed to claim $TASK_ID"
            exit 1
        fi
    else
        echo "⚠️  No actionable tasks found."
    fi
fi