#!/usr/bin/env bash
# arrange-panes.sh - Apply a tmux layout to the current session
#
# Usage:
#   ./scripts/arrange-panes.sh              # Default: tiled (2x2 for 4 panes)
#   ./scripts/arrange-panes.sh tiled        # 2x2 grid
#   ./scripts/arrange-panes.sh even-horizontal
#   ./scripts/arrange-panes.sh even-vertical
#   ./scripts/arrange-panes.sh main-horizontal
#   ./scripts/arrange-panes.sh main-vertical

LAYOUT="${1:-tiled}"

# Discover current session
if [ -n "${TMUX:-}" ]; then
    SESSION=$(tmux display-message -p '#{session_name}')
else
    echo "Error: not inside a tmux session"
    exit 1
fi

PANE_COUNT=$(tmux list-panes -t "$SESSION" -F '#{pane_index}' | wc -l | tr -d ' ')

tmux select-layout -t "$SESSION" "$LAYOUT"
echo "Applied '$LAYOUT' to $SESSION ($PANE_COUNT panes)"
