#!/bin/bash
# DeepSeek via Claude Code
# Uses DeepSeek's Anthropic-compatible API endpoint with Claude Code CLI
# Includes agent mail integration via system prompt

set -e

if [ -z "${DEEPSEEK_API_KEY:-}" ]; then
    echo "Error: DEEPSEEK_API_KEY environment variable not set"
    echo "Please run: export DEEPSEEK_API_KEY='your-api-key'"
    exit 1
fi

# Project configuration
PROJECT_ROOT="${PROJECT_ROOT:-$(pwd)}"
MAIL_PROJECT_KEY="${MAIL_PROJECT_KEY:-$PROJECT_ROOT}"

# Register with mail system — clear stale identity and get fresh name
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/pane-init.sh"
init_pane "DeepSeek Agent"

# Generate or load persistent session ID for this tmux pane (or default)
DEEPSEEK_SESSION_ID=""
if [ -n "${TMUX:-}" ]; then
    TMUX_PANE=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo "")
    if [ -n "$TMUX_PANE" ]; then
        SAFE_PANE=$(echo "$TMUX_PANE" | tr ':.' '-')
        SESSION_ID_FILE="$PROJECT_ROOT/pids/${SAFE_PANE}.deepseek-session"
        if [ -f "$SESSION_ID_FILE" ]; then
            DEEPSEEK_SESSION_ID=$(cat "$SESSION_ID_FILE" 2>/dev/null | tr -d '[:space:]')
        fi
    fi
fi

# If no session ID yet, generate new one
if [ -z "$DEEPSEEK_SESSION_ID" ]; then
    # Generate UUID using python or fallback
    if command -v python3 >/dev/null 2>&1; then
        DEEPSEEK_SESSION_ID=$(python3 -c "import uuid; print(uuid.uuid4())")
    elif command -v uuidgen >/dev/null 2>&1; then
        DEEPSEEK_SESSION_ID=$(uuidgen)
    else
        # Fallback to random string (not ideal but works)
        DEEPSEEK_SESSION_ID=$(head -c 32 /dev/urandom | base64 | tr -dc 'a-f0-9' | head -c 32)
    fi
    # Save to file if we have a SAFE_PANE
    if [ -n "${SESSION_ID_FILE:-}" ]; then
        mkdir -p "$(dirname "$SESSION_ID_FILE")"
        echo "$DEEPSEEK_SESSION_ID" > "$SESSION_ID_FILE"
    fi
fi

# Export session ID for debugging and as custom header for proxy
export DEEPSEEK_SESSION_ID
# Use Claude Code's custom headers support to send session ID to proxy
export ANTHROPIC_CUSTOM_HEADERS="X-Session-ID: ${DEEPSEEK_SESSION_ID}"

# Clear any existing Anthropic credentials
unset ANTHROPIC_API_KEY
unset ANTHROPIC_BASE_URL
unset ANTHROPIC_MODEL
unset ANTHROPIC_SMALL_FAST_MODEL

# Start auto-compact proxy if not already running
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! pgrep -f "deepseek-compact-proxy.py" > /dev/null; then
    echo "Starting auto-compact proxy..."
    if "$SCRIPT_DIR/start-deepseek-proxy.sh" start > /dev/null 2>&1; then
        sleep 2
        # Verify proxy started
        if pgrep -f "deepseek-compact-proxy.py" > /dev/null; then
            echo "✓ Auto-compact proxy started successfully"
        else
            echo "⚠️  Proxy may have failed to start. Check logs: /tmp/deepseek-proxy.log"
        fi
    else
        echo "⚠️  Proxy startup may have failed. Check logs: /tmp/deepseek-proxy.log"
    fi
fi

# Configure Claude Code to use DeepSeek API
export ANTHROPIC_BASE_URL="http://127.0.0.1:5000/anthropic"
export ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY"
export ANTHROPIC_MODEL="deepseek-chat"
export ANTHROPIC_SMALL_FAST_MODEL="deepseek-chat"
export ANTHROPIC_CUSTOM_HEADERS="X-Session-ID: ${DEEPSEEK_SESSION_ID}"
export API_TIMEOUT_MS=600000
export CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1

# Enable Claude Code's built-in auto-compact at 70% threshold
# NOTE: This triggers internal compaction at 70% instead of default 95%
# Proxy still tracks tokens for monitoring but doesn't return 429 errors
export CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=70

# Export for mail helper
export AGENT_NAME
export PROJECT_ROOT
export MAIL_PROJECT_KEY

#######################################
# Claim next bead from BV
#######################################
claim_next_bead() {
    echo "Checking BV for next recommended bead..."

    # Sync beads data
    br sync --flush-only --force >/dev/null 2>&1 || true

    # Get recommendation
    local bv_output
    bv_output=$(bv --robot-next --format json 2>/dev/null || echo "{}")

    local task_id
    task_id=$(echo "$bv_output" | jq -r '.id // empty' 2>/dev/null)

    if [ -z "$task_id" ] || [ "$task_id" = "null" ]; then
        echo "No beads available from BV"
        return 1
    fi

    local task_title
    task_title=$(echo "$bv_output" | jq -r '.title // "Untitled"' 2>/dev/null)
    echo "Recommended bead: $task_id - $task_title"

    # Check if already in progress by someone else
    local task_status
    task_status=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].status // empty' 2>/dev/null)
    if [ "$task_status" = "in_progress" ]; then
        local current_owner
        current_owner=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
        if [ -n "$current_owner" ] && [ "$current_owner" != "$AGENT_NAME" ]; then
            echo "Bead $task_id already claimed by $current_owner"
            return 1
        fi
    fi

    # Claim the bead
    claim_output=$(br update "$task_id" --status in_progress --assignee "$AGENT_NAME" 2>&1)
    claim_status=$?

    if [ $claim_status -eq 0 ]; then
        # Verify the claim succeeded
        verify_assignee=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
        if [ "$verify_assignee" = "$AGENT_NAME" ]; then
            echo "✓ Claimed bead: $task_id"
            echo "$task_id"
            return 0
        else
            echo "Failed to claim bead $task_id: assignee verification failed" >&2
            echo "Expected: $AGENT_NAME, Got: ${verify_assignee:-<none>}" >&2
            return 1
        fi
    else
        echo "Failed to claim bead $task_id:" >&2
        echo "$claim_output" >&2
        return 1
    fi
}

# Try to claim a bead
BEAD_ID=""
if command -v bv >/dev/null 2>&1 && command -v br >/dev/null 2>&1; then
    BEAD_ID=$(claim_next_bead) || BEAD_ID=""
fi

# Export bead ID for hooks
export AGENT_RUNNER_BEAD="${BEAD_ID}"

# Build system prompt
SYSTEM_PROMPT="You are $AGENT_NAME, a DeepSeek agent in a multi-agent tmux environment.
Follow the instructions in CLAUDE.md for mail, beads, and coordination.

IMPORTANT: After a tool call returns its result, continue with NEW text only. NEVER repeat or rephrase text you already output before the tool call. Pick up exactly where you left off."

# Add bead info if we have one
if [ -n "$BEAD_ID" ]; then
    BEAD_INFO=$(br show "$BEAD_ID" --json 2>/dev/null | jq -r '.[0] | "**\(.title)**\nDescription: \(.description // "No description")\nPriority: \(.priority // "unset")"' 2>/dev/null || echo "")
    SYSTEM_PROMPT="$SYSTEM_PROMPT

Work on bead $BEAD_ID.

$BEAD_INFO

Check your inbox first, then complete this task."
fi

# Launch Claude Code with system prompt for mail integration
echo "====================================="
echo "DeepSeek Agent: $AGENT_NAME"
echo "====================================="
echo "Project: $PROJECT_ROOT"
echo "Model: $ANTHROPIC_MODEL"
if [ -n "$BEAD_ID" ]; then
    echo "Bead: $BEAD_ID"
fi
echo "====================================="
echo ""

exec claude --dangerously-skip-permissions \
    --system-prompt "$SYSTEM_PROMPT" \
    "$@"
