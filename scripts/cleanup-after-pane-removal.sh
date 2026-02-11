#!/bin/bash
# Cleanup stale identity files after pane removal
# Called by tmux hook after-kill-pane

SESSION_NAME="${1:-flywheel}"

# Get the working directory of the first pane in the session to determine PROJECT_ROOT
FIRST_PANE=$(tmux list-panes -t "$SESSION_NAME" -F "#{pane_id}" 2>/dev/null | head -1)
if [ -z "$FIRST_PANE" ]; then
    # No panes left, nothing to clean up
    exit 0
fi

PROJECT_ROOT=$(tmux display-message -t "$FIRST_PANE" -p "#{pane_current_path}" 2>/dev/null)
if [ -z "$PROJECT_ROOT" ] || [ ! -d "$PROJECT_ROOT" ]; then
    exit 0
fi

# Source project config and run discovery to clean up stale files
export PROJECT_ROOT
# Detect agent-flywheel root dynamically
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT_FLYWHEEL_ROOT="$(dirname "$SCRIPT_DIR")"
if [ -f "$AGENT_FLYWHEEL_ROOT/scripts/lib/project-config.sh" ]; then
    source "$AGENT_FLYWHEEL_ROOT/scripts/lib/project-config.sh"

    # Kill stale mail monitors whose panes no longer exist
    if [ -d "$PIDS_DIR" ]; then
        # Build set of active safe-pane names from tmux
        active_panes=""
        while IFS= read -r pane_info; do
            [ -n "$pane_info" ] && active_panes="$active_panes $(echo "$pane_info" | tr ':.' '-') "
        done < <(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)

        for pid_file in "$PIDS_DIR"/*.mail-monitor.pid; do
            [ -f "$pid_file" ] || continue
            safe_pane=$(basename "$pid_file" .mail-monitor.pid)
            if ! echo "$active_panes" | grep -q " ${safe_pane} "; then
                # Pane is gone — kill the monitor if still alive and clean up
                local_pid=$(cat "$pid_file" 2>/dev/null)
                [ -n "$local_pid" ] && kill "$local_pid" 2>/dev/null || true
                rm -f "$pid_file"
            fi
        done
    fi

    if [ -f "$AGENT_FLYWHEEL_ROOT/panes/discover.sh" ]; then
        bash "$AGENT_FLYWHEEL_ROOT/panes/discover.sh" --all --quiet 2>/dev/null || true
    fi
    # Renumber remaining panes to maintain sequential Claude numbering
    if [ -f "$AGENT_FLYWHEEL_ROOT/scripts/renumber-panes.sh" ]; then
        bash "$AGENT_FLYWHEEL_ROOT/scripts/renumber-panes.sh" "$SESSION_NAME" >/dev/null 2>&1 || true
    fi
fi
