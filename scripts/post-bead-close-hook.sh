#!/usr/bin/env bash
# post-bead-close-hook.sh - Auto-trigger next-bead.sh after br close
#
# Claude Code PostToolUse hook for Bash commands.
# Detects when an agent runs `br close` and automatically triggers
# next-bead.sh to claim the next bead and /clear context.
#
# The agent does NOT need to remember to run next-bead.sh — this hook
# handles it automatically.
#
# Hook config (in ~/.claude/settings.json):
#   "PostToolUse": [
#     { "matcher": "Bash", "hooks": [{ "type": "command",
#       "command": "\"$CLAUDE_PROJECT_DIR\"/scripts/post-bead-close-hook.sh" }] }
#   ]

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NEXT_BEAD="$SCRIPT_DIR/next-bead.sh"

# Read hook input (JSON on stdin)
INPUT=$(cat)

# Extract the command that was run
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)

# Only act on `br close bd-` commands (not br close --help, etc.)
if ! echo "$COMMAND" | grep -q "br close bd-"; then
    exit 0
fi

# Check that next-bead.sh exists
if [ ! -x "$NEXT_BEAD" ]; then
    exit 0
fi

# Check that we're in tmux (next-bead.sh needs it for /clear)
if [ -z "${TMUX_PANE:-}" ]; then
    exit 0
fi

# Trigger next-bead.sh in the background
# It will claim next bead, wait for prompt, send /clear, send new prompt
"$NEXT_BEAD" >/dev/null 2>&1 &
disown

exit 0
