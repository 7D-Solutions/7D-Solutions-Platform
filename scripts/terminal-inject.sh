#!/usr/bin/env bash
# terminal-inject.sh - Unified terminal injection queue submission API
#
# Queues terminal injections (commands, mail notifications) for delivery
# by the monitor-agent-mail-to-terminal.sh worker.
#
# Usage:
#   ./scripts/terminal-inject.sh --keys "/clear" --literal
#   ./scripts/terminal-inject.sh --keys "Enter"
#   ./scripts/terminal-inject.sh --mail-notification <msg-id>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/project-config.sh"

# Get agent identity
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
AGENT_NAME_LOWER=$(echo "$AGENT_NAME" | tr 'A-Z' 'a-z')

# Queue file (shared with mail monitor)
# Using unified queue file that replaces the old mail-queue
QUEUE_FILE="$PIDS_DIR/${AGENT_NAME_LOWER}.mail-queue"

# Ensure queue file exists
mkdir -p "$PIDS_DIR"
touch "$QUEUE_FILE"

# Parse arguments
INJECTION_TYPE=""
KEYS=""
LITERAL=false
MAIL_MSG_ID=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --keys)
            INJECTION_TYPE="command"
            KEYS="$2"
            shift 2
            ;;
        --literal)
            LITERAL=true
            shift
            ;;
        --mail-notification)
            INJECTION_TYPE="mail"
            MAIL_MSG_ID="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Validate
if [ -z "$INJECTION_TYPE" ]; then
    echo "Usage: $0 --keys <keys> [--literal] | --mail-notification <msg-id>" >&2
    exit 1
fi

# Create queue entry with millisecond precision to prevent deduplication of rapid commands
# macOS doesn't support %N, so use Python for milliseconds
TIMESTAMP=$(python3 -c "from datetime import datetime, timezone; print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3] + 'Z')" 2>/dev/null || date -u +"%Y-%m-%dT%H:%M:%SZ")

if [ "$INJECTION_TYPE" = "command" ]; then
    # Command injection
    MODE="keys"
    if [ "$LITERAL" = true ]; then
        MODE="literal"
    fi

    jq -nc \
        --arg type "command" \
        --arg ts "$TIMESTAMP" \
        --arg keys "$KEYS" \
        --arg mode "$MODE" \
        '{type: $type, timestamp: $ts, payload: {keys: $keys, mode: $mode}}' \
        >> "$QUEUE_FILE" 2>&1

elif [ "$INJECTION_TYPE" = "mail" ]; then
    # Mail notification injection
    jq -nc \
        --arg type "mail" \
        --arg ts "$TIMESTAMP" \
        --arg msg_id "$MAIL_MSG_ID" \
        '{type: $type, timestamp: $ts, payload: {message_id: $msg_id}}' \
        >> "$QUEUE_FILE" 2>&1
fi

exit 0
