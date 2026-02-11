#!/bin/bash
# Renumber Claude agents to match their pane indices
# Usage: renumber-panes.sh [session_name]

SESSION_NAME="${1:-flywheel}"

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT_FLYWHEEL_ROOT="$(dirname "$SCRIPT_DIR")"
source "$SCRIPT_DIR/lib/project-config.sh"

echo "Renumbering panes in session: $SESSION_NAME"
echo ""

# Get all panes in the session, sorted by pane index
PANES=$(tmux list-panes -t "$SESSION_NAME" -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null | sort -t. -k2 -n)

if [ -z "$PANES" ]; then
    echo "Error: No panes found in session '$SESSION_NAME'"
    exit 1
fi

# Counter for sequential numbering
COUNTER=1

echo "=== Renumbering panes ==="
while IFS= read -r pane_id; do
    # Get current info
    CURRENT_LLM_NAME=$(tmux show -pv -t "$pane_id" @llm_name 2>/dev/null || echo "Unknown")
    CURRENT_AGENT_NAME=$(tmux show -pv -t "$pane_id" @agent_name 2>/dev/null || echo "Unknown")

    # Determine agent type (Claude, Codex, or Terminal)
    AGENT_TYPE="Terminal"
    if [[ "$CURRENT_LLM_NAME" =~ ^Claude ]]; then
        AGENT_TYPE="Claude"
    elif [[ "$CURRENT_LLM_NAME" =~ ^Codex ]]; then
        AGENT_TYPE="Codex"
    fi

    # Only renumber Claude and Codex agents, skip terminals
    if [ "$AGENT_TYPE" != "Terminal" ]; then
        NEW_NAME="$AGENT_TYPE $COUNTER"

        echo "$pane_id: '$CURRENT_LLM_NAME' -> '$NEW_NAME' (Agent: $CURRENT_AGENT_NAME)"

        # Update tmux variable
        tmux set-option -p -t "$pane_id" @llm_name "$NEW_NAME" 2>/dev/null || true

        # Update identity file
        SAFE_PANE=$(echo "$pane_id" | tr ':.' '-')
        IDENTITY_FILE="$PANES_DIR/$SAFE_PANE.identity"

        if [ -f "$IDENTITY_FILE" ]; then
            # Use jq to update the name field
            jq --arg name "$NEW_NAME" '.name = $name' "$IDENTITY_FILE" > "${IDENTITY_FILE}.tmp"
            mv "${IDENTITY_FILE}.tmp" "$IDENTITY_FILE"
        fi

        COUNTER=$((COUNTER + 1))
    else
        echo "$pane_id: Skipping (Terminal/Non-agent)"
    fi
done <<< "$PANES"

echo ""
echo "=== Renumbering complete ==="
echo "Updated $(($COUNTER - 1)) agent panes"
echo ""
echo "Current pane layout:"
tmux list-panes -t "$SESSION_NAME" -F "#{pane_index}: #{@llm_name} - #{@agent_name}" 2>/dev/null
