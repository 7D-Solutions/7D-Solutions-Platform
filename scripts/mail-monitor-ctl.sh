#!/bin/bash
# Control script for Agent Mail Monitor (terminal notifications)
# Usage: ./scripts/mail-monitor-ctl.sh [--pane PANE_ID] {start|stop|status|restart}

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/project-config.sh"

PROJECT_KEY="$MAIL_PROJECT_KEY"

# Parse optional --pane parameter
TARGET_PANE=""
while [[ $# -gt 0 ]]; do
    case $1 in
        --pane)
            TARGET_PANE="$2"
            shift 2
            ;;
        *)
            break
            ;;
    esac
done

# Note: Prerequisite checks moved to validate_environment() function for fault tolerance
# This allows help command to work even without tmux, preventing unnecessary exits

# Validate tmux environment and agent identity
# With retry logic for session initialization race conditions
validate_environment() {
    local max_retries=3
    local retry_delay=2
    local attempt=1

    # Use TARGET_PANE if provided (explicit pane parameter), otherwise auto-detect
    if [ -n "$TARGET_PANE" ]; then
        PANE_ID="$TARGET_PANE"
    else
        PANE_ID=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo "")
        if [ -z "$PANE_ID" ]; then
            echo "Error: Not running inside tmux; cannot determine pane"
            return 1
        fi
    fi

    SAFE_PANE=$(echo "$PANE_ID" | tr ':.' '-')

    # Retry logic for agent-name file (handles session init race conditions)
    while [ $attempt -le $max_retries ]; do
        if [ -z "$AGENT_NAME" ] && [ -f "$PIDS_DIR/${SAFE_PANE}.agent-name" ]; then
            AGENT_NAME=$(cat "$PIDS_DIR/${SAFE_PANE}.agent-name")
        fi

        if [ -n "$AGENT_NAME" ]; then
            # Success - set file paths and return
            PID_FILE="$PIDS_DIR/${SAFE_PANE}.mail-monitor.pid"
            LOG_FILE="$LOGS_DIR/${SAFE_PANE}.mail-monitor.log"

            # Log successful validation (with retry info if applicable)
            if [ $attempt -gt 1 ]; then
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] Validation succeeded on attempt $attempt" >> "$LOG_FILE" 2>/dev/null || true
            fi

            return 0
        fi

        # Agent name not found - log and retry if attempts remain
        local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
        echo "[$timestamp] Attempt $attempt/$max_retries: No agent name found for pane $PANE_ID" >&2

        if [ $attempt -lt $max_retries ]; then
            echo "[$timestamp] Waiting ${retry_delay}s before retry..." >&2
            sleep $retry_delay
            attempt=$((attempt + 1))
        else
            # Final failure - provide detailed error
            echo "[$timestamp] Error: Agent name not found after $max_retries attempts" >&2
            echo "Make sure ${PIDS_DIR}/${SAFE_PANE}.agent-name exists." >&2
            echo "This may indicate a session initialization issue." >&2
            return 1
        fi
    done

    return 1
}

# Check if a PID is a live mail monitor process (guards against PID reuse)
is_monitor_process() {
    local pid="$1"
    if ! ps -p "$pid" > /dev/null 2>&1; then
        return 1  # Process doesn't exist
    fi
    # Verify process command contains our monitor script name
    local cmd
    cmd=$(ps -p "$pid" -o command= 2>/dev/null || echo "")
    if echo "$cmd" | grep -q "monitor-agent-mail-to-terminal"; then
        return 0  # It's our monitor
    fi
    return 1  # PID reused by different process
}

start_monitor() {
    # Validate environment before starting
    if ! validate_environment; then
        echo "❌ Environment validation failed"
        return 1
    fi

    # Acquire lock to prevent race conditions (atomic operation)
    local LOCK_FILE="$PIDS_DIR/${SAFE_PANE}.mail-monitor.lock"
    if ! mkdir "$LOCK_FILE" 2>/dev/null; then
        echo "❌ Another monitor is starting, please wait..."
        return 1
    fi
    # Ensure lock is released on exit (success or failure)
    trap "rmdir '$LOCK_FILE' 2>/dev/null || true" RETURN

    if [ -f "$PID_FILE" ]; then
        local pid=$(cat "$PID_FILE")
        if is_monitor_process "$pid"; then
            echo "❌ Monitor already running (PID: $pid)"
            return 1
        else
            # Stale PID file (dead or wrong process), remove it
            rm -f "$PID_FILE"
        fi
    fi

    mkdir -p "$(dirname "$LOG_FILE")"

    # Check monitor script exists before starting (error recovery)
    if [ ! -f "$SCRIPT_DIR/monitor-agent-mail-to-terminal.sh" ]; then
        echo "❌ Monitor script not found: $SCRIPT_DIR/monitor-agent-mail-to-terminal.sh"
        echo "   Please check your installation."
        return 1
    fi

    echo "📬 Starting Agent Mail Monitor (terminal notifications)..."
    if [ -f "$PIDS_DIR/${SAFE_PANE}.agent-name" ]; then
        export AGENT_NAME=$(cat "$PIDS_DIR/${SAFE_PANE}.agent-name")
    fi
    # Pass MONITOR_SAFE_PANE so the monitor can re-resolve identity after agent name changes
    MONITOR_SAFE_PANE="$SAFE_PANE" \
    nohup "$SCRIPT_DIR/monitor-agent-mail-to-terminal.sh" "$AGENT_NAME" > "$LOG_FILE" 2>&1 &
    local pid=$!
    echo "$pid" > "$PID_FILE"

    sleep 1
    if ps -p "$pid" > /dev/null 2>&1; then
        echo "✅ Monitor started (PID: $pid)"
        echo "   Log: $LOG_FILE"
        echo "   Use 'tail -f $LOG_FILE' to watch for messages"
    else
        echo "❌ Failed to start monitor"
        if [ -f "$LOG_FILE" ]; then
            echo "   Last log lines:"
            tail -n 5 "$LOG_FILE"
        fi
        rm -f "$PID_FILE"
        return 1
    fi
}

stop_monitor() {
    # Validate environment before stopping
    if ! validate_environment; then
        echo "❌ Environment validation failed"
        return 1
    fi

    if [ ! -f "$PID_FILE" ]; then
        echo "❌ Monitor not running"
        return 1
    fi

    local pid=$(cat "$PID_FILE")
    if is_monitor_process "$pid"; then
        echo "📭 Stopping Agent Mail Monitor (PID: $pid)..."
        kill "$pid"
        rm -f "$PID_FILE"
        echo "✅ Monitor stopped"
    else
        echo "❌ Monitor not running (stale PID file)"
        rm -f "$PID_FILE"
        return 1
    fi
}

status_monitor() {
    # Validate environment before checking status
    if ! validate_environment; then
        echo "❌ Environment validation failed"
        return 1
    fi

    if [ ! -f "$PID_FILE" ]; then
        echo "📭 Monitor is NOT running"
        return 1
    fi

    local pid=$(cat "$PID_FILE")
    if is_monitor_process "$pid"; then
        echo "📬 Monitor is RUNNING (PID: $pid)"
        echo "   Log: $LOG_FILE"
        if [ -f "$LOG_FILE" ]; then
            echo ""
            echo "Recent activity (last 10 lines):"
            tail -n 10 "$LOG_FILE"
        fi
    else
        echo "📭 Monitor is NOT running (stale PID file)"
        rm -f "$PID_FILE"
        return 1
    fi
}

ensure_monitor() {
    # Idempotent: start only if not already running. For use by hooks.
    if ! validate_environment; then
        return 1
    fi

    if [ -f "$PID_FILE" ]; then
        local pid=$(cat "$PID_FILE")
        if is_monitor_process "$pid"; then
            # Already running — nothing to do
            return 0
        fi
        # Stale PID, clean up
        rm -f "$PID_FILE"
    fi

    # Not running — start it (quiet output for hook usage)
    start_monitor
}

case "${1:-help}" in
    start)
        start_monitor
        ;;
    stop)
        stop_monitor
        ;;
    status)
        status_monitor
        ;;
    ensure)
        ensure_monitor
        ;;
    restart)
        stop_monitor 2>/dev/null || true
        sleep 1
        start_monitor
        ;;
    logs)
        # Validate environment to get LOG_FILE path
        if ! validate_environment; then
            echo "❌ Environment validation failed"
            echo "   Cannot determine log file location without tmux/agent context"
            exit 1
        fi

        if [ -f "$LOG_FILE" ]; then
            tail -f "$LOG_FILE"
        else
            echo "❌ No log file found"
            echo "   Monitor may not be started yet"
            echo "   Log would be at: $LOG_FILE"
            exit 1
        fi
        ;;
    help|*)
        cat << 'HELP'
Agent Mail Monitor Control

Usage:
  ./scripts/mail-monitor-ctl.sh [--pane PANE_ID] <command>

Options:
  --pane PANE_ID    Specify target pane explicitly (e.g., session-name:1.2)
                    If omitted, auto-detects from current tmux pane

Commands:
  start      Start the mail monitor in background
  stop       Stop the mail monitor
  status     Check if monitor is running and show recent activity
  ensure     Start only if not running (idempotent, for hooks)
  restart    Restart the monitor
  logs       Follow the monitor log file (Ctrl+C to exit)

The monitor will check for new messages every 5 seconds and display
them in the terminal when they arrive.

Examples:
  ./scripts/mail-monitor-ctl.sh start
  ./scripts/mail-monitor-ctl.sh --pane kelly-enterprises:1.1 start
  ./scripts/mail-monitor-ctl.sh --pane session:1.2 status

HELP
        ;;
esac
