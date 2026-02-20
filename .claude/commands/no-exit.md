Run this exact Bash command and nothing else:

PANE_ID=$(tmux display-message -t "${TMUX_PANE:-}" -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo ""); SAFE_PANE=$(echo "$PANE_ID" | tr ':.' '-'); PIDS_DIR="$(pwd)/pids"; NO_EXIT_FILE="$PIDS_DIR/${SAFE_PANE}.no-exit"; mkdir -p "$PIDS_DIR"; if [ -f "$NO_EXIT_FILE" ] && grep -q "on" "$NO_EXIT_FILE" 2>/dev/null; then echo "off" > "$NO_EXIT_FILE"; echo "Agent auto-exit: ON for this agent (exits after each bead)"; else echo "on" > "$NO_EXIT_FILE"; echo "Agent auto-exit: OFF for this agent (stays in REPL loop)"; fi
