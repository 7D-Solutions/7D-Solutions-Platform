#!/usr/bin/env bash
# post-bash-bead-track-hook.sh - Auto-write bead tracking file after br create/update
#
# Claude Code PostToolUse hook for Bash commands.
# Detects when an agent runs `br create` or `br update --status in_progress`
# and writes the bead ID to /tmp/agent-bead-{name}.txt.
#
# This catches agents using br directly instead of wrapper scripts like br-start-work.sh.
#
# Hook config (in ~/.claude/settings.json):
#   "PostToolUse": [
#     { "matcher": "Bash", "hooks": [{ "type": "command",
#       "command": "\"$CLAUDE_PROJECT_DIR\"/scripts/post-bash-bead-track-hook.sh" }] }
#   ]

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Read hook input (JSON on stdin)
INPUT=$(cat)

# Extract the command that was run
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)

if [ -z "$COMMAND" ]; then
    exit 0
fi

BEAD_ID=""

if echo "$COMMAND" | grep -qE '\bbr\s+create\b'; then
    # br create: extract bead ID from command output
    # Try tool_output as string, then stringify if object
    OUTPUT=$(echo "$INPUT" | jq -r 'if .tool_output then (.tool_output | tostring) else "" end' 2>/dev/null || echo "")
    if [ -n "$OUTPUT" ] && [ "$OUTPUT" != "null" ]; then
        BEAD_ID=$(echo "$OUTPUT" | grep -oE 'bd-[a-zA-Z0-9]+(\.[a-zA-Z0-9]+)*' | head -1)
    fi

elif echo "$COMMAND" | grep -qE '\bbr\s+update\b.*(-s|--status)[= ]in_progress'; then
    # br update --status in_progress: extract bead ID from the command itself
    BEAD_ID=$(echo "$COMMAND" | grep -oE 'bd-[a-zA-Z0-9]+(\.[a-zA-Z0-9]+)*' | head -1)

else
    # Not a command we care about
    exit 0
fi

# If we couldn't extract a bead ID, exit silently
if [ -z "$BEAD_ID" ]; then
    exit 0
fi

# Get agent name
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "")

if [ -z "$AGENT_NAME" ]; then
    exit 0
fi

# Write bead tracking file
BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"
echo "$BEAD_ID" > "$BEAD_TRACKING_FILE"

exit 0
