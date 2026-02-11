#!/bin/bash
# Pane initialization — clear stale identity and re-register
# Source this from any agent wrapper before launching claude
#
# Requires: SCRIPT_DIR, PROJECT_ROOT, MAIL_PROJECT_KEY to be set
# Sets: AGENT_NAME (from mail server registration)
#
# Usage in a wrapper:
#   source "$SCRIPT_DIR/lib/pane-init.sh"
#   init_pane "Grok Agent"   # description for mail registration

init_pane() {
    local agent_desc="${1:-Development agent}"

    if [ -z "${TMUX:-}" ]; then
        AGENT_NAME="${AGENT_NAME:-UnknownAgent}"
        return 0
    fi

    local tmux_pane
    tmux_pane=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo "")
    if [ -z "$tmux_pane" ]; then
        AGENT_NAME="${AGENT_NAME:-UnknownAgent}"
        return 0
    fi

    local safe_pane
    safe_pane=$(echo "$tmux_pane" | tr ':.' '-')
    local agent_name_file="$PROJECT_ROOT/pids/${safe_pane}.agent-name"
    local identity_file="$PROJECT_ROOT/panes/${safe_pane}.identity"

    # Clear stale identity from previous session
    rm -f "$agent_name_file"

    # Register fresh with the mail system
    echo "Registering agent with mail system..."
    local reg_output
    reg_output=$(MAIL_PROJECT_KEY="$MAIL_PROJECT_KEY" "$SCRIPT_DIR/agent-mail-helper.sh" register "$agent_desc" 2>&1)
    echo "$reg_output"

    # Read the name the mail server assigned
    if [ -f "$agent_name_file" ]; then
        AGENT_NAME=$(cat "$agent_name_file")
    else
        AGENT_NAME="${AGENT_NAME:-UnknownAgent}"
        echo "Warning: Mail registration did not create agent-name file"
        return 1
    fi

    # Sync tmux pane variable
    tmux set-option -p -t "$tmux_pane" @agent_name "$AGENT_NAME" 2>/dev/null || true

    # Sync identity file if it exists
    if [ -f "$identity_file" ] && command -v jq >/dev/null 2>&1; then
        jq --arg name "$AGENT_NAME" '. + {agent_mail_name: $name}' "$identity_file" > "${identity_file}.tmp" \
            && mv "${identity_file}.tmp" "$identity_file"
    fi

    export AGENT_NAME
    return 0
}
