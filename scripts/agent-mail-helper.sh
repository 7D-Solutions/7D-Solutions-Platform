#!/usr/bin/env bash
# Agent Mail Helper Script
# Provides easy commands for agent-to-agent communication

# Note: set -e intentionally omitted for fault tolerance
# This is a core communication library with 26+ jq pipelines that can fail gracefully
# set -e

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENTCORE_ROOT="$(dirname "$SCRIPT_DIR")"
source "$SCRIPT_DIR/lib/project-config.sh"

# Mail server configuration (can be overridden via environment variables)
MAIL_SERVER="${MAIL_SERVER:-http://127.0.0.1:8765}"
MCP_AGENT_MAIL_DIR="${MCP_AGENT_MAIL_DIR:-$HOME/mcp_agent_mail}"
TOKEN_FILE="$MCP_AGENT_MAIL_DIR/.env"

# Use pane-specific identity (not session-wide)
# Use TMUX_PANE for reliable pane detection (doesn't depend on focus)
if [ -n "$TMUX_PANE" ]; then
    PANE_ID=$(tmux display-message -t "$TMUX_PANE" -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
else
    PANE_ID=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
fi
SAFE_PANE=$(echo "$PANE_ID" | tr ':.' '-')
AGENT_NAME_FILE="$PROJECT_ROOT/pids/${SAFE_PANE}.agent-name"
IDENTITY_FILE="$PROJECT_ROOT/panes/${SAFE_PANE}.identity"
GLOBAL_AGENT_NAME_FILE="$AGENTCORE_ROOT/pids/${SAFE_PANE}.agent-name"

# Read receipts tracking file
READ_RECEIPTS_FILE="$PROJECT_ROOT/.beads/mail-read.jsonl"
mkdir -p "$(dirname "$READ_RECEIPTS_FILE")"
touch "$READ_RECEIPTS_FILE" 2>/dev/null || true

# Load token
if [ ! -f "$TOKEN_FILE" ]; then
    echo "Error: Token file not found at $TOKEN_FILE"
    exit 1
fi
TOKEN=$(grep HTTP_BEARER_TOKEN "$TOKEN_FILE" | cut -d'=' -f2)

# Function to make MCP API calls
mcp_call() {
    local method=$1
    local params=$2
    local payload_file=$3

    if [ -n "$payload_file" ]; then
        # Read from file
        curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d "@$payload_file"
    else
        # Use inline params
        curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":$(date +%s)}"
    fi
}

# Resolve agent names to project roots by scanning panes identity files.
# Prints project root on stdout. Return codes: 0=found, 1=not found, 2=ambiguous.
resolve_agent_project() {
    local agent_name="$1"
    local active_matches=()

    # Prefer active tmux pane mapping if available
    if [ -n "$TMUX" ]; then
        while IFS=$'	' read -r pane_path pane_agent; do
            [ -n "$pane_agent" ] || continue
            if [ "$pane_agent" = "$agent_name" ]; then
                # Find project root by walking up to find directory containing panes/
                local project_root="$pane_path"
                while [ "$project_root" != "/" ] && [ "$project_root" != "$HOME" ]; do
                    if [ -d "$project_root/panes" ]; then
                        active_matches+=("$project_root")
                        break
                    fi
                    project_root="$(dirname "$project_root")"
                done
            fi
        done < <(tmux list-panes -a -F "#{pane_current_path}	#{@agent_name}" 2>/dev/null)

        if [ ${#active_matches[@]} -gt 0 ]; then
            # Deduplicate
            local unique=()
            local seen=""
            for p in "${active_matches[@]}"; do
                if [[ " $seen " != *" $p "* ]]; then
                    unique+=("$p")
                    seen="$seen $p"
                fi
            done
            if [ ${#unique[@]} -eq 1 ]; then
                echo "${unique[0]}"
                return 0
            fi
            # Multiple active panes with same name
            return 2
        fi
    fi

    # Fall back to identity files (stale names possible)
    local matches=()
    while IFS= read -r identity_file; do
        local mail_name
        mail_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)
        if [ "$mail_name" = "$agent_name" ]; then
            local project_root
            project_root=$(dirname "$(dirname "$identity_file")")
            matches+=("$project_root")
        fi
    done < <(find "$PROJECT_ROOT" -maxdepth 3 -path "*/panes/*.identity" -type f 2>/dev/null)

    if [ ${#matches[@]} -eq 0 ]; then
        return 1
    fi

    # Deduplicate
    local unique=()
    local seen=""
    for p in "${matches[@]}"; do
        if [[ " $seen " != *" $p "* ]]; then
            unique+=("$p")
            seen="$seen $p"
        fi
    done

    if [ ${#unique[@]} -gt 1 ]; then
        return 2
    fi

    # No active pane, but identity found
    echo "${unique[0]}"
    return 3
}

# Suggest agent names using case-insensitive matching
# Prints matching agent names to stdout, one per line
suggest_agent_names() {
    local query="$1"
    local query_lower=$(echo "$query" | tr '[:upper:]' '[:lower:]')
    local suggestions=()
    local seen=""

    # Search active tmux panes first
    if [ -n "$TMUX" ]; then
        while IFS=$'\t' read -r pane_path pane_agent; do
            [ -n "$pane_agent" ] || continue
            local agent_lower=$(echo "$pane_agent" | tr '[:upper:]' '[:lower:]')
            if [ "$agent_lower" = "$query_lower" ] && [[ " $seen " != *" $pane_agent "* ]]; then
                suggestions+=("$pane_agent")
                seen="$seen $pane_agent "
            fi
        done < <(tmux list-panes -a -F "#{pane_current_path}\t#{@agent_name}" 2>/dev/null)
    fi

    # Search identity files
    while IFS= read -r identity_file; do
        local mail_name
        mail_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)
        [ -n "$mail_name" ] || continue
        local name_lower=$(echo "$mail_name" | tr '[:upper:]' '[:lower:]')
        if [ "$name_lower" = "$query_lower" ] && [[ " $seen " != *" $mail_name "* ]]; then
            suggestions+=("$mail_name")
            seen="$seen $mail_name "
        fi
    done < <(find "$PROJECT_ROOT" -maxdepth 3 -path "*/panes/*.identity" -type f 2>/dev/null)

    # Print results
    for suggestion in "${suggestions[@]}"; do
        echo "$suggestion"
    done
}

# Send message to a specific project key (project root path)
send_message_with_project() {
    local project_key="$1"
    local to_agents="$2"
    local subject="$3"
    local body="$4"
    local importance="${5:-normal}"

    if [ -z "$to_agents" ] || [ -z "$subject" ] || [ -z "$body" ]; then
        echo "Usage: $0 send 'Agent1,Agent2' 'Subject' 'Message body' [importance]"
        exit 1
    fi

    local my_name
    my_name=$(whoami_agent)
    local sender_name="${MAIL_SENDER_NAME:-$my_name}"

    # Ensure project exists
    cat > /tmp/ensure-project.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "ensure_project",
    "arguments": {
      "human_key": "$project_key"
    }
  },
  "id": $(date +%s)
}
EOF
    curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/ensure-project.json >/dev/null 2>&1 || true
    rm -f /tmp/ensure-project.json

    # Ensure sender is registered in target project (register both pane agent and override sender if different)
    cat > /tmp/register-agent.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "register_agent",
    "arguments": {
      "project_key": "$project_key",
      "program": "claude-code",
      "model": "sonnet",
      "name": "$my_name",
      "task_description": "Cross-project messenger"
    }
  },
  "id": $(date +%s)
}
EOF
    curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/register-agent.json >/dev/null 2>&1 || true
    rm -f /tmp/register-agent.json

    if [ "$sender_name" != "$my_name" ]; then
      local sender_program="claude-code"
      local sender_model="sonnet"
      if [ "$sender_name" = "SystemNotify" ]; then
        sender_program="system"
        sender_model="system"
      fi
      cat > /tmp/register-sender.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "register_agent",
    "arguments": {
      "project_key": "$project_key",
      "program": "$sender_program",
      "model": "$sender_model",
      "name": "$sender_name",
      "task_description": "Override sender name for notifications"
    }
  },
  "id": $(date +%s)
}
EOF
      curl -s -X POST "$MAIL_SERVER/mcp" \
          -H "Authorization: Bearer $TOKEN" \
          -H "Content-Type: application/json" \
          -d @/tmp/register-sender.json >/dev/null 2>&1 || true
      rm -f /tmp/register-sender.json
    fi

    # Convert comma-separated to JSON array
    local to_array
    to_array=$(echo "$to_agents" | jq -R 'split(",") | map(gsub("^\\s+|\\s+$";""))')

    # Properly escape subject and body for JSON
    local subject_json
    subject_json=$(echo "$subject" | jq -Rs .)
    local body_json
    body_json=$(echo "$body" | jq -Rs .)

    cat > /tmp/send-msg.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "send_message",
    "arguments": {
      "project_key": "$project_key",
      "sender_name": "$sender_name",
      "to": $to_array,
      "subject": $subject_json,
      "body_md": $body_json,
      "importance": "$importance"
    }
  },
  "id": $(date +%s)
}
EOF

    echo "Sending message from $my_name to $to_agents..."
    local result
    local curl_exit_code
    result=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/send-msg.json)
    curl_exit_code=$?

    # Check curl exit code
    if [ $curl_exit_code -ne 0 ]; then
        echo "Error: curl failed with exit code $curl_exit_code" >&2
        rm -f /tmp/send-msg.json
        return 1
    fi

    # Check for JSON-RPC error in response
    local error_message
    error_message=$(echo "$result" | jq -r '.error.message // empty' 2>/dev/null)
    if [ -n "$error_message" ]; then
        echo "Error: MCP server returned error: $error_message" >&2
        rm -f /tmp/send-msg.json
        return 1
    fi

    local delivery_count
    delivery_count=$(echo "$result" | jq -r '.result.structuredContent.deliveries | map(.payload.to | length) | add // 0' 2>/dev/null)
    if [ -z "$delivery_count" ] || [ "$delivery_count" = "null" ]; then
        delivery_count=0
    fi

    # Check if delivery actually succeeded
    if [ "$delivery_count" -eq 0 ]; then
        echo "Error: Message failed to deliver to any recipients" >&2
        echo "Response: $result" >&2
        rm -f /tmp/send-msg.json
        return 1
    fi

    # Wait for message to be fully processed on server before notifying
    sleep 0.5
    echo "Sent to $delivery_count recipient(s)"

    rm -f /tmp/send-msg.json
}

# Keep agent name in sync with tmux label and identity file.
sync_agent_name() {
    local tmux_name=""
    if [ -n "$TMUX" ] && [ -n "$PANE_ID" ]; then
        tmux_name=$(tmux show -pv -t "${TMUX_PANE:-$PANE_ID}" @agent_name 2>/dev/null || true)
    fi

    local current_name=""
    if [ -f "$AGENT_NAME_FILE" ]; then
        current_name=$(cat "$AGENT_NAME_FILE" 2>/dev/null || true)
    fi

    local chosen_name=""
    if [ -n "$tmux_name" ] && [ "$tmux_name" != "null" ]; then
        chosen_name="$tmux_name"
    elif [ -z "$current_name" ] && [ -f "$GLOBAL_AGENT_NAME_FILE" ]; then
        chosen_name=$(cat "$GLOBAL_AGENT_NAME_FILE" 2>/dev/null || true)
    fi

    if [ -n "$chosen_name" ] && [ "$chosen_name" != "$current_name" ]; then
        mkdir -p "$(dirname "$AGENT_NAME_FILE")"
        echo "$chosen_name" > "$AGENT_NAME_FILE"
        export AGENT_NAME="$chosen_name"

        if [ -f "$IDENTITY_FILE" ]; then
            jq --arg name "$chosen_name" '. + {agent_mail_name: $name}' "$IDENTITY_FILE" > "${IDENTITY_FILE}.tmp"
            mv "${IDENTITY_FILE}.tmp" "$IDENTITY_FILE"
        fi

        if [ -n "$TMUX" ] && [ -n "$PANE_ID" ]; then
            tmux set-option -p -t "${TMUX_PANE:-$PANE_ID}" @agent_name "$chosen_name" 2>/dev/null || true
        fi
    fi
}

# Register agent if not already registered
register() {
    if [ -f "$AGENT_NAME_FILE" ]; then
        echo "Already registered as: $(cat "$AGENT_NAME_FILE")"
        return 0
    fi

    mkdir -p "$(dirname "$AGENT_NAME_FILE")"

    local task_desc="${1:-Development agent}"

    echo "Registering agent..."
    cat > /tmp/agent-reg.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "macro_start_session",
    "arguments": {
      "human_key": "$MAIL_PROJECT_KEY",
      "program": "claude-code",
      "model": "sonnet",
      "task_description": "$task_desc"
    }
  },
  "id": $(date +%s)
}
EOF
    local response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/agent-reg.json)

    local agent_name=$(echo "$response" | jq -r '.result.structuredContent.agent.name')

    if [ "$agent_name" != "null" ] && [ -n "$agent_name" ]; then
        echo "$agent_name" > "$AGENT_NAME_FILE"
        echo "Registered as: $agent_name"
        echo "$agent_name"
    else
        echo "Error: Failed to register"
        echo "$response" | jq .
        exit 1
    fi
}

# Ensure this pane is registered before using mail
ensure_registered() {
    sync_agent_name
    if [ -n "$AGENT_NAME" ]; then
        return 0
    fi

    if [ -f "$AGENT_NAME_FILE" ]; then
        export AGENT_NAME=$(cat "$AGENT_NAME_FILE")
        return 0
    fi

    if [ -n "$PANE_ID" ]; then
        if [ -f "$SCRIPTS_DIR/auto-register-agent.sh" ]; then
            QUIET=true source "$SCRIPTS_DIR/auto-register-agent.sh" || true
        fi
    fi

    if [ -n "$AGENT_NAME" ]; then
        return 0
    fi

    if [ -f "$AGENT_NAME_FILE" ]; then
        export AGENT_NAME=$(cat "$AGENT_NAME_FILE")
        return 0
    fi

    return 1
}

# Get current agent name
whoami_agent() {
    sync_agent_name
    # REMOVED: Environment variable check to prevent manual overrides
    # if [ -n "$AGENT_NAME" ]; then
    #     echo "$AGENT_NAME"
    #     return 0
    # fi

    # Try pane-specific file
    if [ -f "$AGENT_NAME_FILE" ]; then
        cat "$AGENT_NAME_FILE"
        return 0
    fi
    if ensure_registered; then
        echo "$AGENT_NAME"
        return 0
    fi

    # If not in tmux, fall back to any registered agent
    if [ -z "$PANE_ID" ]; then
        FIRST_AGENT=$(ls "$PROJECT_ROOT/pids/"*.agent-name 2>/dev/null | head -1)
        if [ -n "$FIRST_AGENT" ]; then
            cat "$FIRST_AGENT"
            return 0
        fi
    fi

    echo "Error: Not registered in this pane. Run: $0 register"
    exit 1
}

# List all agents in project
list_agents() {
    local slug=$(echo "$MAIL_PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')
    local active_only="${1:-false}"

    cat > /tmp/list-agents.json << EOF
{
  "jsonrpc": "2.0",
  "method": "resources/read",
  "params": {
    "uri": "resource://agents/$slug"
  },
  "id": $(date +%s)
}
EOF

    if [ "$active_only" = "true" ]; then
        # Get list of agents with active tmux panes
        local active_agents=()
        if [ -n "$TMUX" ]; then
            ACTIVE_PANES=$(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
            for identity_file in "$PROJECT_ROOT/panes/"*.identity; do
                [ -f "$identity_file" ] || continue
                local pane=$(jq -r '.pane' "$identity_file" 2>/dev/null)
                if echo "$ACTIVE_PANES" | grep -q "^${pane}$" 2>/dev/null; then
                    local agent_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)
                    [ -n "$agent_name" ] && active_agents+=("$agent_name")
                fi
            done
        fi

        # Filter to only active agents
        local output=""
        output=$(curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d @/tmp/list-agents.json | \
            jq -r '.result.contents[0].text' | \
            jq -r '.agents[] | "\(.name)\t\(.program)\t\(.task_description)"' | \
            while IFS=$'\t' read -r name program task; do
                for active in "${active_agents[@]}"; do
                    if [ "$name" = "$active" ]; then
                        printf "%s\t%s\t%s\n" "$name" "$program" "$task"
                        break
                    fi
                done
            done)
        echo "$output" | column -t -s $'\t'
    else
        # Show all agents
        curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d @/tmp/list-agents.json | \
            jq -r '.result.contents[0].text' | \
            jq -r '.agents[] | "\(.name)\t\(.program)\t\(.task_description)"' | \
            column -t -s $'\t'
    fi
}

# List all agents across all projects known to the MCP server
list_agents_all() {
    local active_only="${1:-false}"
    local include_archive="${2:-false}"
    cat > /tmp/list-projects.json << EOF
{
  "jsonrpc": "2.0",
  "method": "resources/read",
  "params": {
    "uri": "resource://projects"
  },
  "id": $(date +%s)
}
EOF

    local projects_json
    projects_json=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/list-projects.json)

    local err
    err=$(echo "$projects_json" | jq -r '.error.message // empty')
    if [ -n "$err" ]; then
        # Fallback: scan identity files on disk
        local search_root="$HOME/Projects"
        if [ ! -d "$search_root" ]; then
            search_root="$HOME"
        fi
        local active_panes=""
        if [ "$active_only" = "true" ]; then
            active_panes=$(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || true)
        fi
        find "$search_root" -maxdepth 5 -path "*/panes/*.identity" -type f 2>/dev/null | \
            while IFS= read -r identity_file; do
                if [ "$include_archive" != "true" ]; then
                    echo "$identity_file" | grep -q "/archive/" && continue
                fi
                local agent_name
                local agent_type
                local pane
                agent_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)
                [ -n "$agent_name" ] || continue
                agent_type=$(jq -r '.type // empty' "$identity_file" 2>/dev/null)
                pane=$(jq -r '.pane // empty' "$identity_file" 2>/dev/null)
                if [ "$active_only" = "true" ]; then
                    echo "$active_panes" | grep -q "^${pane}$" 2>/dev/null || continue
                fi
                local project_root
                project_root=$(dirname "$(dirname "$identity_file")")
                if [ "$include_archive" != "true" ]; then
                    echo "$project_root" | grep -q "/archive$" && continue
                fi
                printf "%s\t%s\t%s\t%s\n" "$agent_name" "$agent_type" "$pane" "$project_root"
            done | sort -u | column -t -s $'\t'
        return 0
    fi

    local projects
    projects=$(echo "$projects_json" | jq -r '.result.contents[0].text' | jq -c '.[]' 2>/dev/null || true)
    if [ -z "$projects" ]; then
        echo "No projects found."
        return 0
    fi

    local rows=()
    local active_panes=""
    if [ "$active_only" = "true" ]; then
        active_panes=$(tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || true)
    fi
    while IFS= read -r project; do
        [ -z "$project" ] && continue
        local slug
        local human_key
        slug=$(echo "$project" | jq -r '.slug')
        human_key=$(echo "$project" | jq -r '.human_key')

        cat > /tmp/list-project-agents.json << EOF
{
  "jsonrpc": "2.0",
  "method": "resources/read",
  "params": {
    "uri": "resource://project/$slug"
  },
  "id": $(date +%s)
}
EOF

        local proj_json
        proj_json=$(curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d @/tmp/list-project-agents.json)

        local agents
        if [ "$active_only" = "true" ]; then
            agents=$(echo "$proj_json" | jq -r '.result.contents[0].text' | \
                jq -r '.agents[] | "\(.name)\t\(.program)\t\(.task_description)\t\(.pane // "")"' 2>/dev/null || true)
        else
            agents=$(echo "$proj_json" | jq -r '.result.contents[0].text' | \
                jq -r '.agents[] | "\(.name)\t\(.program)\t\(.task_description)\t"' 2>/dev/null || true)
        fi
        if [ -n "$agents" ]; then
            while IFS=$'\t' read -r name program task pane; do
                if [ "$active_only" = "true" ]; then
                    [ -n "$pane" ] || continue
                    echo "$active_panes" | grep -q "^${pane}$" 2>/dev/null || continue
                fi
                rows+=("$name\t$program\t$task\t$human_key")
            done <<< "$agents"
        fi
    done <<< "$projects"

    if [ ${#rows[@]} -eq 0 ]; then
        echo "No agents found."
        return 0
    fi

    printf "%s\n" "${rows[@]}" | column -t -s $'\t'
}

# Send message
send_message() {
    local to_agents="$1"
    local subject="$2"
    local body="$3"
    local importance="${4:-normal}"

    if [ -z "$to_agents" ] || [ -z "$subject" ] || [ -z "$body" ]; then
        echo "Usage: $0 send 'Agent1,Agent2' 'Subject' 'Message body' [importance]"
        exit 1
    fi

    # Group recipients by resolved project key (project root path).
    local IFS=',' recipients=()
    read -r -a recipients <<< "$to_agents"

    # Build group list as "project_root=recipient1,recipient2"
    local groups=()
    for recipient in "${recipients[@]}"; do
        recipient=$(echo "$recipient" | xargs)
        [ -z "$recipient" ] && continue

        local project_root=""
        set +e  # Temporarily disable exit-on-error to read status safely
        project_root=$(resolve_agent_project "$recipient")
        local status=$?
        set +e  # keep relaxed mode; callers handle errors explicitly
        if [ "$status" -eq 0 ]; then
            :
        elif [ "$status" -eq 3 ]; then
            echo "Warning: Recipient '$recipient' has no active pane; routing based on last known identity." >&2
        elif [ "$status" -eq 2 ]; then
            echo "Error: Recipient '$recipient' found in multiple projects. Please disambiguate." >&2
            exit 1
        else
            # status = 1: Agent not found - try fuzzy matching
            echo "Error: Agent '$recipient' not found." >&2
            local suggestions=$(suggest_agent_names "$recipient")
            if [ -n "$suggestions" ]; then
                local count=$(echo "$suggestions" | wc -l | tr -d ' ')
                if [ "$count" -eq 1 ]; then
                    echo "Did you mean: $suggestions?" >&2
                else
                    echo "Did you mean one of these?" >&2
                    echo "$suggestions" | sed 's/^/  - /' >&2
                fi
            else
                echo "No similar agent names found." >&2
                echo "Use '$0 list' to see all available agents." >&2
            fi
            exit 1
        fi

        local updated=false
        local i=0
        while [ $i -lt ${#groups[@]} ]; do
            local entry="${groups[$i]}"
            local key="${entry%%=*}"
            local val="${entry#*=}"
            if [ "$key" = "$project_root" ]; then
                groups[$i]="$key,$val,$recipient"
                updated=true
                break
            fi
            i=$((i+1))
        done
        if [ "$updated" = false ]; then
            groups+=("$project_root=$recipient")
        fi
    done

    # Send one message per project key
    local failed_count=0
    local total_count=${#groups[@]}
    for entry in "${groups[@]}"; do
        local project_root="${entry%%=*}"
        local recipients_csv="${entry#*=}"
        recipients_csv=$(echo "$recipients_csv" | sed 's/^,*//; s/,,*/,/g')
        if ! send_message_with_project "$project_root" "$recipients_csv" "$subject" "$body" "$importance"; then
            failed_count=$((failed_count + 1))
        fi
    done

    # Report overall status if any deliveries failed
    if [ $failed_count -gt 0 ]; then
        echo "Warning: $failed_count of $total_count deliveries failed" >&2
        return 1
    fi

    # Auto-notify disabled - monitors handle notifications to avoid duplicates
    # (Previously sent immediate tmux notifications here, but that caused double
    # notifications when combined with monitor-based notifications)
}

# Check inbox
# Generate a message hash for tracking (from + subject)
generate_message_hash() {
    local from="$1"
    local subject="$2"
    printf "%s" "${from}::${subject}" | md5 2>/dev/null || printf "%s" "${from}::${subject}" | md5sum 2>/dev/null | cut -d' ' -f1
}

# Delete messages by IDs via MCP API
delete_messages_by_ids() {
    local my_name="$1"
    shift
    local message_ids=("$@")

    if [ ${#message_ids[@]} -eq 0 ]; then
        return 0
    fi

    # Build JSON array of message IDs
    local ids_json=$(printf '%s\n' "${message_ids[@]}" | jq -R . | jq -s .)

    cat > /tmp/delete-messages.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "delete_messages",
    "arguments": {
      "project_key": "$MAIL_PROJECT_KEY",
      "agent_name": "$my_name",
      "message_ids": $ids_json
    }
  },
  "id": $(date +%s)
}
EOF

    local result=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/delete-messages.json 2>/dev/null)

    rm -f /tmp/delete-messages.json

    # Check for success
    local deleted_count=$(echo "$result" | jq -r '.result.structuredContent.deleted // 0' 2>/dev/null)
    if [ "$deleted_count" -gt 0 ]; then
        echo "[mail-cleanup] Deleted $deleted_count message(s)" >&2
    fi
}

# Cleanup old read messages when threshold is reached
# Keeps only the N newest read messages, deletes the rest
cleanup_old_read_messages() {
    local my_name="$1"
    local keep_count="${2:-2}"
    local threshold="${3:-5}"

    # Count read messages for this agent
    if [ ! -f "$READ_RECEIPTS_FILE" ] || [ ! -s "$READ_RECEIPTS_FILE" ]; then
        return 0
    fi

    local read_count=$(jq -c --arg agent "$my_name" \
        'select(.agent == $agent)' \
        "$READ_RECEIPTS_FILE" 2>/dev/null | wc -l | tr -d ' ')

    if [ "$read_count" -lt "$threshold" ]; then
        return 0
    fi

    echo "[mail-cleanup] Read message count ($read_count) reached threshold ($threshold). Cleaning up..." >&2

    # Fetch inbox to get message IDs
    cat > /tmp/cleanup-inbox.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "fetch_inbox",
    "arguments": {
      "project_key": "$MAIL_PROJECT_KEY",
      "agent_name": "$my_name",
      "limit": 100,
      "include_bodies": false
    }
  },
  "id": $(date +%s)
}
EOF

    local response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/cleanup-inbox.json 2>/dev/null)

    rm -f /tmp/cleanup-inbox.json

    # Build list of read message IDs with their timestamps
    local read_messages_json=$(echo "$response" | jq -c '.result.structuredContent.result[]' 2>/dev/null | \
        while IFS= read -r msg; do
            local msg_id=$(echo "$msg" | jq -r '.id // empty')
            local from=$(echo "$msg" | jq -r '.from // empty')
            local subject=$(echo "$msg" | jq -r '.subject // empty' | tr -d '\n')

            if [ -n "$msg_id" ] && [ -n "$from" ] && [ -n "$subject" ]; then
                local read_ts=$(get_read_timestamp "$my_name" "$from" "$subject")
                if [ -n "$read_ts" ] && [ "$read_ts" != "null" ]; then
                    jq -n --arg id "$msg_id" --arg ts "$read_ts" '{id: $id, read_at: $ts}'
                fi
            fi
        done | jq -s .)

    # Sort by read_at timestamp (oldest first) and get IDs to delete
    local to_delete_count=$((read_count - keep_count))
    if [ "$to_delete_count" -le 0 ]; then
        return 0
    fi

    local ids_to_delete=$(echo "$read_messages_json" | \
        jq -r --arg count "$to_delete_count" \
        'sort_by(.read_at) | .[0:($count | tonumber)] | .[].id' 2>/dev/null)

    if [ -z "$ids_to_delete" ]; then
        echo "[mail-cleanup] No messages to delete" >&2
        return 0
    fi

    # Convert to array and delete
    local delete_ids=()
    while IFS= read -r id; do
        [ -n "$id" ] && delete_ids+=("$id")
    done <<< "$ids_to_delete"

    if [ ${#delete_ids[@]} -gt 0 ]; then
        delete_messages_by_ids "$my_name" "${delete_ids[@]}"

        # Clean up read receipts for deleted messages (by matching IDs would require fetching again)
        # For now, keep the receipts - they're small and harmless
        # Future enhancement: track message IDs in receipts for cleanup
    fi
}

# Mark a message as read
mark_message_read() {
    local my_name="${1:-$(whoami_agent)}"
    local from="$2"
    local subject="$3"

    if [ -z "$from" ] || [ -z "$subject" ]; then
        return 0
    fi

    local msg_hash=$(generate_message_hash "$from" "$subject")

    # Create read receipt record
    local read_at=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    local receipt=$(jq -n \
        --arg agent "$my_name" \
        --arg msg_hash "$msg_hash" \
        --arg from "$from" \
        --arg subject "$subject" \
        --arg read_at "$read_at" \
        '{agent: $agent, message_hash: $msg_hash, from: $from, subject: $subject, read_at: $read_at}')

    # Append to read receipts file (deduplicate by removing old entries for same message+agent)
    if [ -f "$READ_RECEIPTS_FILE" ] && [ -s "$READ_RECEIPTS_FILE" ]; then
        jq -c --arg agent "$my_name" --arg hash "$msg_hash" \
            'select(.agent != $agent or .message_hash != $hash)' \
            "$READ_RECEIPTS_FILE" > "${READ_RECEIPTS_FILE}.tmp" 2>/dev/null || true
        mv "${READ_RECEIPTS_FILE}.tmp" "$READ_RECEIPTS_FILE" 2>/dev/null || true
    fi
    echo "$receipt" >> "$READ_RECEIPTS_FILE"

    # Trigger cleanup if threshold reached (asynchronously to avoid blocking)
    # Keep 2 newest read messages when count reaches 5
    (cleanup_old_read_messages "$my_name" 2 5 &) 2>/dev/null || true
}

# Check if a message has been read
is_message_read() {
    local my_name="${1:-$(whoami_agent)}"
    local from="$2"
    local subject="$3"

    if [ ! -f "$READ_RECEIPTS_FILE" ] || [ -z "$from" ] || [ -z "$subject" ]; then
        return 1
    fi

    # Try new hash (without newline)
    local msg_hash=$(generate_message_hash "$from" "$subject")
    if jq -e --arg agent "$my_name" --arg hash "$msg_hash" \
        'select(.agent == $agent and .message_hash == $hash)' \
        "$READ_RECEIPTS_FILE" >/dev/null 2>&1; then
        return 0
    fi

    # Try old hash (with newline) for backward compatibility
    local old_hash
    if command -v md5 >/dev/null 2>&1; then
        old_hash=$(printf "%s\n" "${from}::${subject}" | md5 2>/dev/null)
    else
        old_hash=$(printf "%s\n" "${from}::${subject}" | md5sum 2>/dev/null | cut -d' ' -f1)
    fi

    if [ -n "$old_hash" ]; then
        jq -e --arg agent "$my_name" --arg hash "$old_hash" \
            'select(.agent == $agent and .message_hash == $hash)' \
            "$READ_RECEIPTS_FILE" >/dev/null 2>&1
    else
        return 1
    fi
}

# Get read timestamp for a message
get_read_timestamp() {
    local my_name="${1:-$(whoami_agent)}"
    local from="$2"
    local subject="$3"

    if [ ! -f "$READ_RECEIPTS_FILE" ] || [ -z "$from" ] || [ -z "$subject" ]; then
        echo ""
        return
    fi

    # Try new hash (without newline) first
    local msg_hash=$(generate_message_hash "$from" "$subject")
    local timestamp=$(jq -r --arg agent "$my_name" --arg hash "$msg_hash" \
        'select(.agent == $agent and .message_hash == $hash) | .read_at' \
        "$READ_RECEIPTS_FILE" 2>/dev/null | head -1)

    if [ -n "$timestamp" ] && [ "$timestamp" != "null" ]; then
        echo "$timestamp"
        return
    fi

    # Try old hash (with newline) for backward compatibility
    local old_hash
    if command -v md5 >/dev/null 2>&1; then
        old_hash=$(printf "%s\n" "${from}::${subject}" | md5 2>/dev/null)
    else
        old_hash=$(printf "%s\n" "${from}::${subject}" | md5sum 2>/dev/null | cut -d' ' -f1)
    fi

    if [ -n "$old_hash" ]; then
        jq -r --arg agent "$my_name" --arg hash "$old_hash" \
            'select(.agent == $agent and .message_hash == $hash) | .read_at' \
            "$READ_RECEIPTS_FILE" 2>/dev/null | head -1
    else
        echo ""
    fi
}

check_inbox() {
    local limit="${1:-20}"
    local unread_only="${2:-false}"
    local my_name=$(whoami_agent)

    cat > /tmp/check-inbox.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "fetch_inbox",
    "arguments": {
      "project_key": "$MAIL_PROJECT_KEY",
      "agent_name": "$my_name",
      "limit": $limit,
      "include_bodies": true
    }
  },
  "id": $(date +%s)
}
EOF

    local response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/check-inbox.json)

    local messages=$(echo "$response" | jq -r '.result.structuredContent.result // empty')

    if [ -z "$messages" ] || [ "$messages" = "null" ] || [ "$messages" = "[]" ]; then
        echo "Inbox for $my_name:"
        echo "===================="
        echo "(No messages)"
        return
    fi

    # Count unread messages
    local total_count=$(echo "$response" | jq -r '.result.structuredContent.result | length')
    unread_count=0
    while IFS= read -r msg; do
        local from=$(echo "$msg" | jq -r '.from')
        local subject=$(echo "$msg" | jq -r '.subject' | tr -d '\n')
        if [ -n "$from" ] && [ -n "$subject" ]; then
            if ! is_message_read "$my_name" "$from" "$subject"; then
                unread_count=$((unread_count + 1))
            fi
        fi
    done < <(echo "$response" | jq -c '.result.structuredContent.result[]' 2>/dev/null)

    echo "Inbox for $my_name:"
    echo "===================="
    if [ $unread_count -gt 0 ]; then
        echo "($unread_count unread of $total_count messages)"
        echo ""
    fi

    # Display messages with read indicators
    echo "$response" | jq -c '.result.structuredContent.result[]' | while IFS= read -r msg; do
        local from=$(echo "$msg" | jq -r '.from')
        local subject=$(echo "$msg" | jq -r '.subject' | tr -d '\n')
        local body=$(echo "$msg" | jq -r '.body_md')
        local importance=$(echo "$msg" | jq -r '.importance')
        local is_read=false
        local indicator="●"  # Unread indicator

        if [ -n "$from" ] && [ -n "$subject" ] && is_message_read "$my_name" "$from" "$subject"; then
            is_read=true
            indicator="○"  # Read indicator
        fi

        # Skip read messages if unread_only is true
        if [ "$unread_only" = "true" ] && [ "$is_read" = "true" ]; then
            continue
        fi

        echo "[$indicator $importance] From: $from | $subject"
        echo "$body"
        # Mark message as read after displaying
        mark_message_read "$my_name" "$from" "$subject"
        echo "---"
    done
}

#######################################
# Check product inbox (across all linked projects)
# Arguments:
#   $1 - Product UID
#   $2 - Limit (default: 20)
#######################################
check_inbox_product() {
    local product_uid="$1"
    local limit="${2:-20}"
    local my_name=$(whoami_agent)

    cat > /tmp/check-inbox-product.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "fetch_inbox_product",
    "arguments": {
      "product_key": "$product_uid",
      "agent_name": "$my_name",
      "limit": $limit,
      "include_bodies": true
    }
  },
  "id": $(date +%s)
}
EOF

    local response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @/tmp/check-inbox-product.json)

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo "Error: $error"
        return 1
    fi

    local messages=$(echo "$response" | jq -r '.result.structuredContent.result // empty')

    if [ -z "$messages" ] || [ "$messages" = "null" ] || [ "$messages" = "[]" ]; then
        echo "Product Inbox for $my_name (Product: $product_uid):"
        echo "===================================================="
        echo "(No messages)"
        return
    fi

    # Count messages
    local total_count=$(echo "$response" | jq -r '.result.structuredContent.result | length')

    echo "Product Inbox for $my_name (Product: $product_uid):"
    echo "===================================================="
    echo "($total_count message(s) across all linked projects)"
    echo ""

    # Display messages with project context
    echo "$response" | jq -c '.result.structuredContent.result[]' 2>/dev/null | while IFS= read -r line; do
        local from=$(echo "$line" | jq -r '.from // "unknown"')
        local subject=$(echo "$line" | jq -r '.subject // "(no subject)"')
        local body=$(echo "$line" | jq -r '.body // ""')
        local importance=$(echo "$line" | jq -r '.importance // "normal"')
        local project_slug=$(echo "$line" | jq -r '.project_slug // "unknown"')

        # Check if message has been read
        local is_read="false"
        if is_message_read "$my_name" "$from" "$subject"; then
            is_read="true"
        fi

        # Determine indicator
        local indicator="●"  # unread
        if [ "$is_read" = "true" ]; then
            indicator="○"  # read
        fi

        echo "[$indicator $importance] [$project_slug] From: $from | $subject"
        echo "$body"
        # Mark message as read after displaying
        mark_message_read "$my_name" "$from" "$subject"
        echo "---"
    done
}

# Main command dispatcher
case "${1:-help}" in
    register)
        register "${2:-Development agent}"
        ;;
    whoami)
        whoami_agent
        ;;
    list)
        if [ "$2" = "--active" ]; then
            list_agents "true"
        elif [ "$2" = "--all" ]; then
            all_active="false"
            include_archive="false"
            for arg in "$3" "$4"; do
                [ "$arg" = "--active" ] && all_active="true"
                [ "$arg" = "--include-archive" ] && include_archive="true"
            done
            list_agents_all "$all_active" "$include_archive"
        else
            list_agents "false"
        fi
        ;;
    send)
        send_message "$2" "$3" "$4" "$5"
        ;;
    inbox)
        check_inbox "${2:-20}" "false"
        ;;
    inbox-product)
        # Product-scoped inbox (requires product_uid)
        if [ -z "$2" ]; then
            echo "Error: Product UID required"
            echo "Usage: $0 inbox-product <product_uid> [limit]"
            exit 1
        fi
        check_inbox_product "$2" "${3:-20}"
        ;;
    unread)
        check_inbox "${2:-20}" "true"
        ;;
    mark-read)
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Error: Sender and subject required"
            echo "Usage: $0 mark-read <from> <subject>"
            exit 1
        fi
        my_name=$(whoami_agent)
        mark_message_read "$my_name" "$2" "$3"
        echo "Marked message from $2 as read"
        ;;
    mark-all-read)
        my_name=$(whoami_agent)
        # Fetch recent messages and mark them all as read
        cat > /tmp/check-inbox-all.json << EOF
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "fetch_inbox",
    "arguments": {
      "project_key": "$MAIL_PROJECT_KEY",
      "agent_name": "$my_name",
      "limit": ${2:-50},
      "include_bodies": false
    }
  },
  "id": $(date +%s)
}
EOF
        response=$(curl -s -X POST "$MAIL_SERVER/mcp" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Content-Type: application/json" \
            -d @/tmp/check-inbox-all.json)

        count=0
        echo "$response" | jq -c '.result.structuredContent.result[] | {from: .from, subject: .subject}' 2>/dev/null | while IFS= read -r msg; do
            from=$(echo "$msg" | jq -r '.from // ""')
            subject=$(echo "$msg" | jq -r '.subject // ""')
            if [ -n "$from" ] && [ -n "$subject" ]; then
                mark_message_read "$my_name" "$from" "$subject"
                count=$((count + 1))
            fi
        done
        echo "Marked all messages as read"
        ;;
    broadcast)
        # Broadcast message to multiple agents
        if [ $# -lt 3 ]; then
            echo "Error: broadcast requires agents, channel, and message"
            echo "Usage: $0 broadcast <agents|all> <channel> <message> [importance]"
            exit 1
        fi

        local agents="$2"
        local channel="$3"
        local message="$4"
        local importance="${5:-normal}"

        # If "all" is specified, get all active agents
        if [ "$agents" = "all" ]; then
            agents=$(list_agents "true" | tail -n +1 | awk '{print $1}' | tr '\n' ',' | sed 's/,$//')
            if [ -z "$agents" ]; then
                echo "Error: No active agents found"
                exit 1
            fi
        fi

        # Send with broadcast prefix
        local subject="[BROADCAST:$channel] System Message"
        send_message "$agents" "$subject" "$message" "$importance"
        ;;
    test-message)
        send_message "cloudybadger" "Test Subject" "This is a test message from CloudyBadger" "normal"
        ;;
    help|*)
        cat << 'HELP'
Agent Mail Helper

Usage:
  ./scripts/agent-mail-helper.sh <command> [args]

Commands:
  register [description]           Register this agent with mail system
  whoami                          Show current agent name
  list [--active|--all [--active] [--include-archive]] List agents in project (or across all projects)
  send <to> <subject> <body>      Send message to agents (comma-separated)
  broadcast <agents|all> <channel> <message> [importance] Broadcast to multiple agents
  inbox [limit]                   Check inbox (default: 20 messages)
  inbox-product <product_uid> [limit] Check inbox across all projects in product
  unread [limit]                  Show only unread messages
  mark-read <msg_id> [from] [subj] Mark a message as read
  mark-all-read [limit]           Mark all messages as read

Read Receipts:
  Messages are marked with ● (unread) or ○ (read) indicators
  Read status is tracked locally in .beads/mail-read.jsonl

Examples:
  ./scripts/agent-mail-helper.sh register "Frontend developer"
  ./scripts/agent-mail-helper.sh list
  ./scripts/agent-mail-helper.sh list --active
  ./scripts/agent-mail-helper.sh list --all
  ./scripts/agent-mail-helper.sh send "CloudyBridge,DustyStream" "Status update" "Feature complete"
  ./scripts/agent-mail-helper.sh broadcast "all" "maintenance" "System restarting in 5min"
  ./scripts/agent-mail-helper.sh broadcast "Agent1,Agent2,Agent3" "qa" "New test suite available"
  ./scripts/agent-mail-helper.sh inbox 10
  ./scripts/agent-mail-helper.sh unread
  ./scripts/agent-mail-helper.sh mark-all-read

HELP
        ;;
esac
