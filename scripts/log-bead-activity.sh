#!/bin/bash
# Bead activity logging for adoption metrics
# Logs to .beads/agent-activity.jsonl

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LOG_FILE="${PROJECT_DIR}/.beads/agent-activity.jsonl"

# Ensure log file exists
mkdir -p "$(dirname "$LOG_FILE")"
touch "$LOG_FILE"

# Function to log an event
log_bead_event() {
    local agent="$1"
    local bead_id="$2"
    local action="$3"
    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    # Create JSON entry (compact format for JSONL)
    local entry
    entry=$(jq -c -n \
        --arg ts "$timestamp" \
        --arg agent "$agent" \
        --arg bead "$bead_id" \
        --arg action "$action" \
        '{timestamp: $ts, agent: $agent, bead_id: $bead, action: $action}')

    echo "$entry" >> "$LOG_FILE"
    # Optional: echo to stderr for debugging
    echo "[bead-log] $timestamp $agent $bead_id $action" >&2
}

# Function to get current agent name
get_agent_name() {
    "$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown"
}

# If script is called directly with parameters, log event
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    if [ $# -lt 3 ]; then
        echo "Usage: $0 <bead_id> <action> [agent_name]" >&2
        echo "  action: claim, create, edit_allowed, edit_blocked, close, commit" >&2
        exit 1
    fi
    bead_id="$1"
    action="$2"
    agent="${3:-$(get_agent_name)}"
    log_bead_event "$agent" "$bead_id" "$action"
fi