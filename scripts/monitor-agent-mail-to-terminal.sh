#!/bin/bash
# Agent Mail Monitor - Sends notifications directly to terminal via tmux
# Usage: ./scripts/monitor-agent-mail-to-terminal.sh [agent_name] [poll_interval]

# Note: set -e intentionally omitted for fault tolerance
# Monitor should keep running even if curl/jq operations fail
# set -e

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/project-config.sh"

# Mail server configuration (can be overridden via environment variables)
MAIL_SERVER="${MAIL_SERVER:-http://127.0.0.1:8765}"
MCP_AGENT_MAIL_DIR="${MCP_AGENT_MAIL_DIR:-$HOME/mcp_agent_mail}"
TOKEN_FILE="$MCP_AGENT_MAIL_DIR/.env"
PROJECT_KEY="$MAIL_PROJECT_KEY"
AGENT_NAME="${1:-$AGENT_NAME}"
POLL_INTERVAL="${2:-5}"

# Determine our safe-pane identity (stable across agent name changes)
# This is the key used for PID files and identity files.
# Sources: MONITOR_SAFE_PANE (set by mail-monitor-ctl.sh), or live tmux detection.
SAFE_PANE="${MONITOR_SAFE_PANE:-}"
if [ -z "$SAFE_PANE" ] && [ -n "${TMUX_PANE:-}" ]; then
    PANE_ID=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo "")
    [ -n "$PANE_ID" ] && SAFE_PANE=$(echo "$PANE_ID" | tr ':.' '-')
fi

# Find agent name if not provided — use safe-pane to look it up
if [ -z "$AGENT_NAME" ] && [ -n "$SAFE_PANE" ]; then
    AGENT_FILE="$PIDS_DIR/${SAFE_PANE}.agent-name"
    if [ -f "$AGENT_FILE" ]; then
        AGENT_NAME=$(cat "$AGENT_FILE")
    fi
fi

if [ -z "$AGENT_NAME" ]; then
    echo "Error: No agent name provided and couldn't detect from pane"
    echo "Usage: $0 <agent_name> [poll_interval]"
    exit 1
fi

# Resolve pane ID for current agent name.
# Searches identity files by agent name, falls back to safe-pane lookup.
# Returns 0 on success (MY_PANE set), 1 on failure.
resolve_pane() {
    local max_retries="${1:-10}"
    local retry_delay=2
    local attempt=1

    while [ $attempt -le $max_retries ]; do
        # Method 1: Search identity files by agent_mail_name
        for identity_file in "$PANES_DIR/"*.identity; do
            if [ -f "$identity_file" ]; then
                local mail_name
                mail_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)
                if [ "$mail_name" = "$AGENT_NAME" ]; then
                    MY_PANE=$(jq -r '.pane' "$identity_file")
                    # Verify the pane actually exists in tmux
                    if verify_pane; then
                        [ $attempt -gt 1 ] && echo "[$(date '+%Y-%m-%d %H:%M:%S')] Resolved pane on attempt $attempt"
                        return 0
                    fi
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Found identity for $AGENT_NAME but pane $MY_PANE doesn't exist"
                    MY_PANE=""
                fi
            fi
        done

        # Method 2: If we know our safe-pane, check if agent name changed
        if [ -n "$SAFE_PANE" ]; then
            local agent_file="$PIDS_DIR/${SAFE_PANE}.agent-name"
            local identity_file="$PANES_DIR/${SAFE_PANE}.identity"
            if [ -f "$agent_file" ]; then
                local new_name
                new_name=$(cat "$agent_file")
                if [ -n "$new_name" ] && [ "$new_name" != "$AGENT_NAME" ]; then
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Agent name changed: $AGENT_NAME -> $new_name"
                    AGENT_NAME="$new_name"
                    # Re-derive tracking files for new name
                    LAST_MSG_FILE="$PIDS_DIR/$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z').last-msg-id"
                    QUEUE_FILE="$PIDS_DIR/$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z').mail-queue"
                    [ ! -f "$LAST_MSG_FILE" ] && echo "0" > "$LAST_MSG_FILE"
                    touch "$QUEUE_FILE" 2>/dev/null || true
                fi
            fi
            if [ -f "$identity_file" ]; then
                MY_PANE=$(jq -r '.pane' "$identity_file" 2>/dev/null)
                if [ -n "$MY_PANE" ] && [ "$MY_PANE" != "null" ] && verify_pane; then
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Resolved pane via safe-pane fallback: $MY_PANE"
                    return 0
                fi
                MY_PANE=""
            fi
        fi

        echo "[$(date '+%Y-%m-%d %H:%M:%S')] Attempt $attempt/$max_retries: Could not resolve pane for $AGENT_NAME"
        if [ $attempt -lt $max_retries ]; then
            sleep $retry_delay
            attempt=$((attempt + 1))
        else
            return 1
        fi
    done
    return 1
}

# Verify MY_PANE still exists in tmux. Returns 0 if valid, 1 if stale.
verify_pane() {
    # Ask tmux for the actual session:window.pane of the target and compare.
    # tmux falls back to current pane for bad targets (exit 0), so we must
    # verify the returned value matches what we asked for.
    local actual
    actual=$(tmux display-message -t "$MY_PANE" -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
    [ "$actual" = "$MY_PANE" ] && return 0
    return 1
}

# Initial pane resolution (with longer retry for startup race conditions)
MY_PANE=""
echo "[$(date '+%Y-%m-%d %H:%M:%S')] Searching for identity file for agent: $AGENT_NAME"

if ! resolve_pane 15; then
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Error: Could not find pane for agent $AGENT_NAME after 15 attempts"
    echo "Identity files checked in: $PANES_DIR"
    exit 1
fi

# Load token
if [ ! -f "$TOKEN_FILE" ]; then
    echo "Error: Token file not found at $TOKEN_FILE"
    exit 1
fi
TOKEN=$(grep HTTP_BEARER_TOKEN "$TOKEN_FILE" | cut -d'=' -f2)

# Track last seen message
LAST_MSG_FILE="$PIDS_DIR/$(echo $AGENT_NAME | tr 'A-Z' 'a-z').last-msg-id"
if [ ! -f "$LAST_MSG_FILE" ]; then
    echo "0" > "$LAST_MSG_FILE"
fi

# Queue file for messages waiting to be delivered (when user is typing)
QUEUE_FILE="$PIDS_DIR/$(echo $AGENT_NAME | tr 'A-Z' 'a-z').mail-queue"
touch "$QUEUE_FILE" 2>/dev/null || true

echo "📬 Mail-to-Terminal Monitor started"
echo "   Agent: $AGENT_NAME"
echo "   Pane: $MY_PANE"
echo "   Polling every ${POLL_INTERVAL}s"
echo "   Queue file: $QUEUE_FILE"
echo "   ✨ Input detection enabled - messages queue when you're typing"
echo "   ✨ Queued messages retry every ${POLL_INTERVAL}s until terminal is clear"
echo "   Press Ctrl+C to stop"
echo ""

# Function to check if user has pending input (typing but not submitted)
# Returns 0 if pending input detected, 1 if clear
# Scans last lines of pane for the ❯ prompt with text after it.
# The prompt line moves depending on pane height and status bar, so we
# don't rely on cursor position — just find ❯ wherever it is.
has_pending_input() {
    # Capture the last 10 lines of the pane (prompt is always near the bottom)
    local bottom
    bottom=$(tmux capture-pane -t "$MY_PANE" -p 2>/dev/null | tail -10)

    # Safety check
    if [[ -z "$bottom" ]]; then
        return 1  # No pending input (fail-safe)
    fi

    # Look for the Claude Code input prompt: ❯ at line start (with optional spaces)
    # If text follows ❯, user is typing. If just ❯ + spaces, prompt is empty.
    # Use only lines starting with optional-whitespace then ❯ to avoid matching
    # ❯ appearing inside command output.
    local prompt_line
    prompt_line=$(echo "$bottom" | grep '^[[:space:]]*❯' | tail -1)
    if [ -z "$prompt_line" ]; then
        return 1  # No prompt visible — claude is working
    fi
    local after_prompt
    after_prompt=$(echo "$prompt_line" | sed 's/^[[:space:]]*❯//')
    # Check for non-whitespace after prompt (ignore regular space AND non-breaking space \xc2\xa0)
    local trimmed
    trimmed=$(echo "$after_prompt" | sed $'s/[\t \xc2\xa0]//g')
    if [ -n "$trimmed" ]; then
        return 0  # User is typing on the prompt line
    fi

    return 1  # No pending input
}

# Note: is_pane_idle() removed - we now simply retry delivery every poll interval
# until the terminal is clear (no pending input). This is simpler and more reliable
# than trying to track pane activity timestamps.

# Function to add message to queue
queue_message() {
    local message="$1"
    # Write as JSON for unified queue format
    local timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    jq -n \
        --arg type "mail" \
        --arg ts "$timestamp" \
        --arg msg "$message" \
        '{type: $type, timestamp: $ts, payload: {text: $msg}}' \
        >> "$QUEUE_FILE"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Queued message (user typing): $message"
}

# Function to flush queued messages to terminal
# Handles both mail notifications and command injections from unified queue
# Includes TTL (time-to-live) and idempotency
flush_queue() {
    if [ ! -s "$QUEUE_FILE" ]; then
        return  # Queue empty
    fi

    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Flushing queued items..."

    local current_epoch=$(date -u +%s)
    local ttl_seconds=300  # 5 minutes TTL for queue items
    local processed_items=()  # For idempotency tracking
    local temp_queue=$(mktemp)

    while IFS= read -r item; do
        [ -z "$item" ] && continue

        # Try to parse as JSON first (new format)
        local item_type=$(echo "$item" | jq -r '.type // "legacy"' 2>/dev/null)

        # Check TTL for JSON items
        if [ "$item_type" != "legacy" ]; then
            local timestamp=$(echo "$item" | jq -r '.timestamp // ""')
            if [ -n "$timestamp" ]; then
                # Strip milliseconds if present (e.g., 2026-02-09T22:45:02.879Z -> 2026-02-09T22:45:02Z)
                local timestamp_no_ms=$(echo "$timestamp" | sed 's/\.[0-9]\{3\}Z$/Z/')
                local item_epoch=$(date -u -d "$timestamp_no_ms" +%s 2>/dev/null || date -j -u -f "%Y-%m-%dT%H:%M:%SZ" "$timestamp_no_ms" +%s 2>/dev/null || echo "0")
                local age=$((current_epoch - item_epoch))

                if [ "$age" -gt "$ttl_seconds" ]; then
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Skipping stale item (age: ${age}s): $item_type"
                    continue
                fi
            fi

            # Idempotency: check if we've already processed this exact item
            local item_hash=$(echo "$item" | md5)
            if [[ " ${processed_items[@]} " =~ " ${item_hash} " ]]; then
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] Skipping duplicate item: $item_type"
                continue
            fi
            processed_items+=("$item_hash")
        fi

        if [ "$item_type" = "mail" ]; then
            # Mail notification
            local text=$(echo "$item" | jq -r '.payload.text // .payload.message_id // ""')
            if [ -n "$text" ]; then
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] Delivering mail: $text"
                tmux send-keys -t "$MY_PANE" "$text"
                sleep 0.5
                tmux send-keys -t "$MY_PANE" C-m
                sleep 0.5
            fi
        elif [ "$item_type" = "command" ]; then
            # Command injection
            local keys=$(echo "$item" | jq -r '.payload.keys')
            local mode=$(echo "$item" | jq -r '.payload.mode // "keys"')
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] Delivering command: $keys (mode: $mode)"

            if [ "$mode" = "literal" ]; then
                tmux send-keys -t "$MY_PANE" -l "$keys"
            else
                tmux send-keys -t "$MY_PANE" "$keys"
            fi
            sleep 0.5
        elif [ "$item_type" = "legacy" ]; then
            # Backward compatibility: plain text message (old format)
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] Delivering legacy: $item"
            tmux send-keys -t "$MY_PANE" "$item"
            sleep 0.5
            tmux send-keys -t "$MY_PANE" C-m
            sleep 0.5
        fi
    done < "$QUEUE_FILE"

    # Clear the queue
    > "$QUEUE_FILE"
    rm -f "$temp_queue"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Queue flushed (processed: ${#processed_items[@]} unique items)"
}

# Function to send notification to terminal (as input for Claude Code)
# Now checks for pending input first
send_to_terminal() {
    local message="$1"

    # Check if user is actively typing
    if has_pending_input; then
        echo "[$(date '+%Y-%m-%d %H:%M:%S')] Pending input detected - queueing message"
        queue_message "$message"
        return
    fi

    # Send as actual input that Claude will see
    tmux send-keys -t "$MY_PANE" "$message"
    sleep 0.5
    tmux send-keys -t "$MY_PANE" C-m
}

# Function to check for new messages
check_new_messages() {
    cat > /tmp/monitor-inbox-$AGENT_NAME.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "fetch_inbox",
    "arguments": {
      "project_key": "$PROJECT_KEY",
      "agent_name": "$AGENT_NAME",
      "limit": 50,
      "include_bodies": true
    }
  },
  "id": $(date +%s)
}
EOF

    local response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/monitor-inbox-$AGENT_NAME.json)

    local last_seen=$(cat "$LAST_MSG_FILE" 2>/dev/null || echo "0")
    local messages=$(echo "$response" | jq -r '.result.structuredContent.result // []')

    if [ "$messages" != "[]" ] && [ "$messages" != "null" ]; then
        local newest_id=$(echo "$response" | jq -r '.result.structuredContent.result[0].id // 0')

        # Detect message ID reset (server restart)
        # If newest_id is significantly less than last_seen, the server was likely restarted
        # Reset tracking to avoid missing messages
        if [ -n "$newest_id" ] && [ -n "$last_seen" ] && [ "$newest_id" -lt "$last_seen" ]; then
            local id_diff=$((last_seen - newest_id))
            if [ "$id_diff" -gt 100 ]; then
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] NOTICE: Message ID reset detected (was $last_seen, now $newest_id). Server likely restarted. Resetting tracking." >&2
                last_seen=0
                echo "0" > "$LAST_MSG_FILE"
            fi
        fi

        if [ -n "$newest_id" ] && [ -n "$last_seen" ] && [ "$newest_id" -gt "$last_seen" ]; then
            # DEBUG: Log the check
            echo "[DEBUG] newest_id=$newest_id, last_seen=$last_seen" >&2

            # Update last_seen IMMEDIATELY to prevent duplicate notifications
            # This must happen before sending notifications to avoid race condition
            echo "$newest_id" > "$LAST_MSG_FILE"

            # DEBUG: Confirm file updated
            echo "[DEBUG] Updated LAST_MSG_FILE to: $(cat "$LAST_MSG_FILE")" >&2

            # Format and send new messages to terminal
            local notification=$(echo "$response" | jq -r --arg last "$last_seen" '
                .result.structuredContent.result[] |
                select(.id > ($last | tonumber)) |
                "📨 NEW MAIL from \(.from)"
            ')

            # DEBUG: Show what notifications will be sent
            echo "[DEBUG] Notification count: $(echo "$notification" | wc -l)" >&2
            echo "[DEBUG] Notifications: $notification" >&2

            if [ -n "$notification" ]; then
                # Print to this script's output
                echo ""
                echo "╔════════════════════════════════════════════╗"
                echo "║  NEW MESSAGE RECEIVED                      ║"
                echo "╚════════════════════════════════════════════╝"
                echo "$notification"

                # Send visual notification to the terminal pane
                echo "$notification" | while IFS= read -r line; do
                    echo "[DEBUG] Sending line: $line" >&2
                    send_to_terminal "$line"
                done
            fi
        fi
    fi
}

# Trap for clean exit
cleanup() {
    echo ""
    echo "📭 Mail monitor stopped"
    # Flush any remaining queued messages before exit
    if [ -s "$QUEUE_FILE" ]; then
        echo "   Flushing remaining queued messages..."
        flush_queue
    fi
    exit 0
}
trap cleanup INT TERM

# Main loop
PANE_CHECK_COUNTER=0
PANE_CHECK_INTERVAL=12  # Verify pane every ~60s (12 * 5s poll)

while true; do
    # Periodically verify pane and agent identity
    PANE_CHECK_COUNTER=$((PANE_CHECK_COUNTER + 1))
    if [ $PANE_CHECK_COUNTER -ge $PANE_CHECK_INTERVAL ]; then
        PANE_CHECK_COUNTER=0

        # Check if agent name changed (re-registration after restart)
        if [ -n "$SAFE_PANE" ]; then
            local_agent_file="$PIDS_DIR/${SAFE_PANE}.agent-name"
            if [ -f "$local_agent_file" ]; then
                current_name=$(cat "$local_agent_file")
                if [ -n "$current_name" ] && [ "$current_name" != "$AGENT_NAME" ]; then
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Agent name changed: $AGENT_NAME -> $current_name"
                    AGENT_NAME="$current_name"
                    LAST_MSG_FILE="$PIDS_DIR/$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z').last-msg-id"
                    QUEUE_FILE="$PIDS_DIR/$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z').mail-queue"
                    [ ! -f "$LAST_MSG_FILE" ] && echo "0" > "$LAST_MSG_FILE"
                    touch "$QUEUE_FILE" 2>/dev/null || true
                    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Now monitoring mail for: $AGENT_NAME"
                fi
            fi
        fi

        # Verify target pane still exists in tmux
        if ! verify_pane; then
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] Target pane $MY_PANE no longer exists, re-resolving..."
            if resolve_pane 3; then
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] Re-resolved to pane: $MY_PANE (agent: $AGENT_NAME)"
            else
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] Could not re-resolve pane. Exiting so ensure can restart us."
                exit 1
            fi
        fi
    fi

    # Check for new messages
    check_new_messages

    # Try to flush queue if terminal is clear (no pending input)
    # This retries every poll interval until successful
    if [ -s "$QUEUE_FILE" ]; then
        if ! has_pending_input; then
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] Terminal clear - flushing queue"
            flush_queue
        else
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] Queue waiting (user still typing)..."
        fi
    fi

    sleep "$POLL_INTERVAL"
done
