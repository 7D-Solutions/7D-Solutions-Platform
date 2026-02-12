#!/usr/bin/env bash
# next-bead.sh - Transition to next bead with context clear
#
# Called by claude (via Bash tool) after closing a bead.
# Claims the next bead, then sends /clear + new prompt to the tmux pane
# so claude starts the next bead with fresh context.
#
# Usage (from within claude):
#   ./scripts/next-bead.sh
#
# Part of: Autonomous Agent Lifecycle System (bd-3u96)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Lock file prevents double-trigger (hook + agent both calling this)
LOCK_FILE="/tmp/next-bead-${TMUX_PANE:-$$}.lock"
if [ -f "$LOCK_FILE" ]; then
    lock_age=$(( $(date +%s) - $(stat -f %m "$LOCK_FILE" 2>/dev/null || echo 0) ))
    if [ "$lock_age" -lt 120 ]; then
        echo "Transition already in progress (lock age: ${lock_age}s). Skipping."
        exit 0
    fi
fi
touch "$LOCK_FILE"

# Get agent identity for tracking file
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")

# Claim next bead
br sync --flush-only --force >/dev/null 2>&1 || true

bv_output=$(bv --robot-next --format json 2>/dev/null || echo "{}")
bead_id=$(echo "$bv_output" | jq -r '.id // empty' 2>/dev/null)

prompt=""

if [ -z "$bead_id" ] || [ "$bead_id" = "null" ]; then
    echo "No beads available. Clearing context anyway."
    # Clear stale tracking file so enforcement doesn't think we still have a bead
    rm -f "/tmp/agent-bead-${AGENT_NAME}.txt"
    prompt="No beads are available right now. Check your inbox for work: \$PROJECT_ROOT/scripts/agent-mail-helper.sh inbox"
else
    bead_title=$(echo "$bv_output" | jq -r '.title // "untitled"' 2>/dev/null)

    # Claim it
    claim_output=$(br update "$bead_id" --status in_progress --owner "$AGENT_NAME" --assignee "$AGENT_NAME" 2>&1)
    claim_status=$?
    if [ $claim_status -ne 0 ]; then
        echo "⚠️  Warning: Failed to claim bead $bead_id (br update failed)" >&2
        echo "$claim_output" >&2
        echo "   Continuing anyway - check assignee manually if needed" >&2
    fi

    # Verify the claim succeeded
    verify_assignee=$(br show "$bead_id" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
    if [ "$verify_assignee" != "$AGENT_NAME" ]; then
        echo "❌ ERROR: Failed to claim bead $bead_id" >&2
        echo "   Expected assignee: $AGENT_NAME" >&2
        echo "   Actual assignee: ${verify_assignee:-<none>}" >&2
        echo "   Another agent may have claimed it first." >&2
        echo "   Clearing stale tracking file..." >&2
        rm -f "/tmp/agent-bead-${AGENT_NAME}.txt"
        rm -f "$LOCK_FILE"
        exit 1
    fi

    # Update tracking file (only if claim verified)
    echo "$bead_id" > "/tmp/agent-bead-${AGENT_NAME}.txt"

    # Get bead details for the prompt
    bead_json=$(br show "$bead_id" --json 2>/dev/null || echo "[]")
    description=$(echo "$bead_json" | jq -r '.[0].description // ""' 2>/dev/null)
    priority=$(echo "$bead_json" | jq -r '.[0].priority // ""' 2>/dev/null)

    prompt="Work on bead $bead_id: $bead_title."
    [ -n "$description" ] && prompt="$prompt Description: $description"
    [ -n "$priority" ] && prompt="$prompt Priority: $priority"
    prompt="$prompt Check inbox first, then complete this task."

    echo "Claimed bead $bead_id: $bead_title"
fi

# Check if /clear is disabled via env var or flag file
NO_CLEAR_FILE="$SCRIPT_DIR/../.no-clear"
if [ "${AGENT_NO_CLEAR:-0}" = "1" ] || { [ -f "$NO_CLEAR_FILE" ] && grep -q "on" "$NO_CLEAR_FILE" 2>/dev/null; }; then
    echo "Skipping /clear (AGENT_NO_CLEAR or .no-clear flag set)"
    [ -n "$bead_id" ] && echo "Next bead: $bead_id"
    [ -n "$prompt" ] && echo "Prompt: $prompt"
    rm -f "$LOCK_FILE"
    exit 0
fi

echo "Clearing context..."

# Get the tmux pane we're running in
pane="${TMUX_PANE:-}"
if [ -z "$pane" ]; then
    echo "Not in a tmux pane. Run /clear manually."
    [ -n "$bead_id" ] && echo "  br show $bead_id"
    exit 0
fi

# Helper: wait for claude input prompt (❯) to be stable in pane
# Checks that ❯ is visible AND pane content hasn't changed for 3 consecutive checks.
# This avoids sending keys while claude is still rendering output.
wait_for_prompt() {
    local last_capture=""
    local stable=0
    for i in $(seq 1 90); do
        local current
        current=$(tmux capture-pane -t "$pane" -p 2>/dev/null)
        if echo "$current" | tail -5 | grep -q "❯"; then
            if [ "$current" = "$last_capture" ]; then
                stable=$((stable + 1))
                if [ $stable -ge 3 ]; then
                    return 0
                fi
            else
                stable=0
            fi
            last_capture="$current"
        else
            stable=0
            last_capture=""
        fi
        sleep 1
    done
    return 1
}

# Helper: wait for mail queue to be empty
# Ensures pending mail notifications are delivered before /clear
# Prevents race condition where notifications arrive after /clear
wait_for_mail_queue_empty() {
    local pids_dir="$SCRIPT_DIR/../pids"
    local agent_name_lower
    agent_name_lower=$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z')
    local queue_file="$pids_dir/${agent_name_lower}.mail-queue"

    # If queue file doesn't exist, nothing to wait for
    if [ ! -f "$queue_file" ]; then
        return 0
    fi

    # Wait up to 30 seconds for queue to be empty
    for i in $(seq 1 30); do
        if [ ! -s "$queue_file" ]; then
            # Queue is empty
            return 0
        fi
        sleep 1
    done

    # Timeout - proceed anyway (queue might be stuck)
    return 0
}

# Background: interrupt agent, wait for idle, send /clear, then send new bead assignment
(
    # Interrupt the agent if it's still working (Escape stops current operation)
    "$SCRIPT_DIR/terminal-inject.sh" --keys "Escape"
    sleep 2

    wait_for_prompt

    # Wait for mail queue to be empty before /clear
    # This prevents notifications from arriving after context is cleared
    wait_for_mail_queue_empty

    # Extra settle time after prompt stabilizes
    sleep 3

    # Queue commands via unified terminal injection queue
    # The mail monitor will deliver them when terminal is clear

    # /clear: first Enter opens autocomplete dropdown
    "$SCRIPT_DIR/terminal-inject.sh" --keys "/clear"
    "$SCRIPT_DIR/terminal-inject.sh" --keys "Enter"
    sleep 2

    # Second Enter selects + executes /clear
    "$SCRIPT_DIR/terminal-inject.sh" --keys "Enter"

    # Wait for /clear to complete and prompt to stabilize again
    sleep 3
    wait_for_prompt

    # Send prompt text with literal mode to prevent key interpretation
    "$SCRIPT_DIR/terminal-inject.sh" --keys "$prompt" --literal

    # Wait for paste bracket to close before submitting
    sleep 3
    "$SCRIPT_DIR/terminal-inject.sh" --keys "Enter"
) &
disown

exit 0
