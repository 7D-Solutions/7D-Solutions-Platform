#!/usr/bin/env bash
# Session Stop Hook - Remind agents to update bead status
# Called by Claude Code on Stop events (when Claude finishes responding)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Read hook input from stdin
INPUT=$(cat)

# Check for active bead
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"

if [ -f "$BEAD_TRACKING_FILE" ]; then
    BEAD_ID=$(cat "$BEAD_TRACKING_FILE")

    # Check if bead is still in_progress
    BEAD_STATUS=$(br show "$BEAD_ID" --json 2>/dev/null | jq -r '.[0].status // empty' 2>/dev/null || true)

    if [ "$BEAD_STATUS" = "in_progress" ]; then
        echo "" >&2
        echo "📋 Active bead: $BEAD_ID (in_progress)" >&2
        echo "   If work is complete, remember to:" >&2
        echo "   → br close '$BEAD_ID'   (mark done)" >&2
        echo "   → Or update status if pausing" >&2
    fi
fi

# Always exit 0 - this is advisory only
exit 0
