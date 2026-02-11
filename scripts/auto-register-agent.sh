#!/bin/bash
# Auto-register agent with mail system (pane-specific)
# Source this in your shell init or run at session start

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/project-config.sh"

cd "$PROJECT_ROOT" || exit 1

# Allow quiet mode for non-interactive callers
QUIET="${QUIET:-false}"
# Show progress indicators even when QUIET=true (default: true)
SHOW_PROGRESS="${SHOW_PROGRESS:-true}"
log() {
    if [ "$QUIET" = "true" ]; then
        return 0
    fi
    echo "$1"
}

progress_log() {
    local msg="$1"
    if [ "$QUIET" != "true" ]; then
        printf "%s\r" "$msg"
        return 0
    fi
    if [ "$SHOW_PROGRESS" = "true" ] && [ -t 1 ] && [ -w /dev/tty ]; then
        printf "%s\r" "$msg" >/dev/tty
    fi
}

# Get pane-specific identifier (prefer TMUX_PANE when available)
if [ -n "$TMUX_PANE" ]; then
    PANE_ID=$(tmux display-message -t "$TMUX_PANE" -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
else
    PANE_ID=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
fi
if [ -z "$PANE_ID" ]; then
    log "✗ Not running in tmux"
    return 1 2>/dev/null || exit 1
fi

SAFE_PANE=$(echo "$PANE_ID" | tr ':.' '-')
AGENT_NAME_FILE="$PROJECT_ROOT/pids/${SAFE_PANE}.agent-name"

# Collect active agent names to check for conflicts
get_active_agent_names() {
    tmux list-panes -a -F "#{@agent_name}" 2>/dev/null \
        | sed '/^$/d' \
        | tr '[:upper:]' '[:lower:]' \
        | sort -u
}

check_name_conflict() {
    local name="$1"
    local current_pane="$2"
    # Get all panes with this agent name, excluding current pane
    local conflicting_panes
    conflicting_panes=$(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index} #{@agent_name}" 2>/dev/null \
        | grep -i " $name$" \
        | grep -v "^$current_pane " \
        | cut -d' ' -f1)

    if [ -n "$conflicting_panes" ]; then
        return 0  # Conflict found
    else
        return 1  # No conflict
    fi
}

# If already registered, check for name conflicts
if [ -f "$AGENT_NAME_FILE" ]; then
    EXISTING_NAME=$(cat "$AGENT_NAME_FILE")

    # Check if this name conflicts with another pane
    if check_name_conflict "$EXISTING_NAME" "$PANE_ID"; then
        log "⚠️  Name conflict detected: '$EXISTING_NAME' is already in use by another pane"
        log "🔄 Generating new unique name..."
        # Move conflicting name file aside
        mv "$AGENT_NAME_FILE" "${AGENT_NAME_FILE}.conflict-$(date +%s)"
        # Continue to registration below to generate new unique name
    else
        # No conflict, use existing name
        export AGENT_NAME="$EXISTING_NAME"
        log "✓ Agent registered as: $AGENT_NAME (pane: $PANE_ID)"

        # Update tmux pane metadata if not already set
        if [ -n "$TMUX_PANE" ]; then
            CURRENT_AGENT_NAME=$(tmux display-message -t "$TMUX_PANE" -p "#{@agent_name}" 2>/dev/null)
            if [ -z "$CURRENT_AGENT_NAME" ]; then
                tmux set-option -p -t "$TMUX_PANE" @agent_name "$AGENT_NAME" 2>/dev/null || true
            fi
        fi

        return 0 2>/dev/null || exit 0
    fi
fi

# Auto-register
log "🔄 Registering new agent for pane $PANE_ID..."

# Get pane index for friendly name
PANE_INDEX=$(echo "$PANE_ID" | grep -oE '\.[0-9]+$' | tr -d '.')

# Check if @llm_name is already set (by startup script) to determine agent type
if [ -n "$TMUX_PANE" ]; then
    EXISTING_LLM_NAME=$(tmux display-message -t "$TMUX_PANE" -p "#{@llm_name}" 2>/dev/null)
else
    # When called from discover.sh, TMUX_PANE may not be set
    EXISTING_LLM_NAME=$(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index} #{@llm_name}" 2>/dev/null | grep "^$PANE_ID " | cut -d' ' -f2-)
fi

# Set TASK_DESC based on agent type from @llm_name
if [[ "$EXISTING_LLM_NAME" == DeepSeek* ]]; then
    TASK_DESC="$EXISTING_LLM_NAME - DeepSeek Agent"
elif [[ "$EXISTING_LLM_NAME" == Codex* ]]; then
    TASK_DESC="$EXISTING_LLM_NAME - Codex Agent"
elif [[ "$EXISTING_LLM_NAME" == Grok* ]]; then
    TASK_DESC="$EXISTING_LLM_NAME - Grok Agent"
elif [[ "$EXISTING_LLM_NAME" == Claude* ]]; then
    TASK_DESC="$EXISTING_LLM_NAME - Claude Code Session"
else
    # Fallback if @llm_name not set
    TASK_DESC="Claude $PANE_INDEX - Claude Code Session"
fi

# Register using helper script (modified to accept pane ID)
mkdir -p "$(dirname "$AGENT_NAME_FILE")"

# Mail server configuration (can be overridden via environment variables)
MAIL_SERVER="${MAIL_SERVER:-http://127.0.0.1:8765}"
MCP_AGENT_MAIL_DIR="${MCP_AGENT_MAIL_DIR:-$HOME/mcp_agent_mail}"
TOKEN_FILE="$MCP_AGENT_MAIL_DIR/.env"
if [ -f "$TOKEN_FILE" ]; then
    TOKEN=$(grep HTTP_BEARER_TOKEN "$TOKEN_FILE" | cut -d'=' -f2)
else
    log "✗ Token file not found at $TOKEN_FILE"
    return 1 2>/dev/null || exit 1
fi

generate_candidate_name() {
    if [ ! -d "$MCP_AGENT_MAIL_DIR/src" ]; then
        log "✗ MCP agent mail library not found at $MCP_AGENT_MAIL_DIR/src"
        log "  Set MCP_AGENT_MAIL_DIR or install mcp_agent_mail to enable name generation."
        return 1
    fi
    local python_bin="python"
    if ! command -v "$python_bin" >/dev/null 2>&1; then
        python_bin="python3"
    fi
    if ! command -v "$python_bin" >/dev/null 2>&1; then
        log "✗ Python not found (need python or python3 for name generation)"
        return 1
    fi
    PYTHONPATH="$MCP_AGENT_MAIL_DIR/src" "$python_bin" - <<'PY'
from mcp_agent_mail.utils import generate_agent_name
print(generate_agent_name())
PY
}

request_unique_agent_name() {
    local active_names="$1"
    local max_attempts=50
    local attempt=1

    while [ "$attempt" -le "$max_attempts" ]; do
        progress_log "Allocating unique agent name... ($attempt/$max_attempts)"
        local candidate
        candidate=$(generate_candidate_name 2>/dev/null) || candidate=""
        if [ -z "$candidate" ]; then
            log "✗ Failed to generate candidate agent name"
            return 1
        fi

        local candidate_lower
        candidate_lower=$(echo "$candidate" | tr '[:upper:]' '[:lower:]')
        if echo "$active_names" | grep -qx "$candidate_lower"; then
            attempt=$((attempt + 1))
            continue
        fi

        cat > /tmp/agent-reg-${SAFE_PANE}.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "create_agent_identity",
    "arguments": {
      "project_key": "$MAIL_PROJECT_KEY",
      "program": "claude-code",
      "model": "sonnet",
      "name_hint": "$candidate",
      "task_description": "$TASK_DESC"
    }
  },
  "id": $(date +%s)
}
EOF

        RESPONSE=$(curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d @/tmp/agent-reg-${SAFE_PANE}.json)

        local err_msg
        err_msg=$(echo "$RESPONSE" | jq -r '.error.message // empty')
        if [ -n "$err_msg" ]; then
            attempt=$((attempt + 1))
            continue
        fi

        AGENT_NAME=$(echo "$RESPONSE" | jq -r '.result.structuredContent.name // .result.structuredContent.agent.name // .result.name // empty')
        if [ -n "$AGENT_NAME" ] && [ "$AGENT_NAME" != "null" ]; then
            return 0
        fi

        attempt=$((attempt + 1))
    done

    return 1
}

ACTIVE_AGENT_NAMES=$(get_active_agent_names)
if ! request_unique_agent_name "$ACTIVE_AGENT_NAMES"; then
    log "✗ Failed to allocate a unique agent name after 50 attempts"
    return 1 2>/dev/null || exit 1
fi

if [ "$AGENT_NAME" != "null" ] && [ -n "$AGENT_NAME" ]; then
    # Move old file if it exists to avoid conflicts
    if [ -f "$AGENT_NAME_FILE" ]; then
        mv "$AGENT_NAME_FILE" "${AGENT_NAME_FILE}.old"
    fi

    # Write agent name to file
    echo "$AGENT_NAME" > "$AGENT_NAME_FILE"

    # Verify the write succeeded
    if [ ! -f "$AGENT_NAME_FILE" ] || [ "$(cat "$AGENT_NAME_FILE")" != "$AGENT_NAME" ]; then
        log "✗ ERROR: Failed to write agent name file: $AGENT_NAME_FILE"
        return 1 2>/dev/null || exit 1
    fi

    export AGENT_NAME
    log "✓ Registered as: $AGENT_NAME"


    # Also update tmux pane title
    if [ -n "$TMUX_PANE" ]; then
        tmux set-option -p -t "$TMUX_PANE" @agent_name "$AGENT_NAME" 2>/dev/null || true

        # Check if @llm_name is already set (by startup script or discover.sh)
        EXISTING_LLM_NAME=$(tmux display-message -t "$TMUX_PANE" -p "#{@llm_name}" 2>/dev/null)

        if [ -n "$EXISTING_LLM_NAME" ] && [[ "$EXISTING_LLM_NAME" =~ ^(Claude|DeepSeek|Codex|Grok)\ [0-9]+$ ]]; then
            # Preserve existing @llm_name set by startup script (includes DeepSeek via Claude Code)
            log "Preserving existing @llm_name: $EXISTING_LLM_NAME"
        else
            # No existing @llm_name or invalid format, detect from command
            MY_INDEX=$(tmux display-message -t "$TMUX_PANE" -p "#{pane_index}" 2>/dev/null)
            MY_CMD=$(tmux display-message -t "$TMUX_PANE" -p "#{pane_current_command}" 2>/dev/null)
            if [[ "$MY_CMD" == "claude" ]]; then
                LLM_NAME="Claude $MY_INDEX"
            elif [[ "$MY_CMD" == *"codex"* ]]; then
                # Codex CLI detected
                LLM_NAME="Codex $MY_INDEX"
            elif [[ "$MY_CMD" == "python"* ]] || [[ "$MY_CMD" == "aider" ]]; then
                # Check if it's actually aider by looking at process args
                MY_TTY=$(tmux display-message -t "$TMUX_PANE" -p "#{pane_tty}" 2>/dev/null)
                if [ -n "$MY_TTY" ] && lsof -t "$MY_TTY" 2>/dev/null | xargs ps -p 2>/dev/null | grep -q "aider"; then
                    LLM_NAME="Codex $MY_INDEX"
                else
                    LLM_NAME="Terminal $MY_INDEX"
                fi
            else
                LLM_NAME="Terminal $MY_INDEX"
            fi
            tmux set-option -p -t "$TMUX_PANE" @llm_name "$LLM_NAME" 2>/dev/null || true
        fi
    fi
    # Also update pane identity file with agent mail name
    IDENTITY_FILE="$PROJECT_ROOT/panes/${SAFE_PANE}.identity"
    if [ -f "$IDENTITY_FILE" ]; then
        # Add agent_mail_name to identity with flock protection to prevent race conditions
        LOCK_FILE="$PROJECT_ROOT/panes/.${SAFE_PANE}.lock"

        # Use flock if available (prefer Homebrew version on macOS)
        FLOCK_CMD="flock"
        if [ ! -x "$(command -v flock)" ] && [ -x "/opt/homebrew/opt/util-linux/bin/flock" ]; then
            FLOCK_CMD="/opt/homebrew/opt/util-linux/bin/flock"
        fi

        {
            # Acquire exclusive lock (wait max 2 seconds)
            if ! $FLOCK_CMD -x -w 2 200; then
                log "✗ WARNING: Failed to acquire lock for identity file update"
                return 1 2>/dev/null || exit 1
            fi

            # Add agent_mail_name to identity
            jq --arg name "$AGENT_NAME" '. + {agent_mail_name: $name}' "$IDENTITY_FILE" > "${IDENTITY_FILE}.tmp"
            if [ $? -ne 0 ]; then
                log "✗ ERROR: Failed to update identity file with jq"
                rm -f "${IDENTITY_FILE}.tmp"
                return 1 2>/dev/null || exit 1
            fi

            mv "${IDENTITY_FILE}.tmp" "$IDENTITY_FILE"
            if [ $? -ne 0 ]; then
                log "✗ ERROR: Failed to move updated identity file"
                rm -f "${IDENTITY_FILE}.tmp"
                return 1 2>/dev/null || exit 1
            fi

        } 200>"$LOCK_FILE"
    fi
else
    log "✗ Failed to register agent"
    if [ "$QUIET" != "true" ]; then
        echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
    fi
    return 1 2>/dev/null || exit 1
fi
