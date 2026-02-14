#!/bin/bash
# Visual session manager using fzf
# Works on both Mac and Windows (WSL/Git Bash)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
STATE_DIR="$PROJECT_ROOT/.session-state"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
GRAY='\033[0;90m'
CYAN='\033[0;36m'
NC='\033[0m'
BOLD='\033[1m'

# Create state directory for session resurrection
mkdir -p "$STATE_DIR"

# Ensure mail server is running
ensure_mail_server() {
    local MCP_AGENT_MAIL_DIR="${MCP_AGENT_MAIL_DIR:-$HOME/mcp_agent_mail}"
    local PIDS_DIR="$PROJECT_ROOT/pids"
    local PID_FILE="$PIDS_DIR/mail-server.pid"

    # Check if mail server directory exists
    if [ ! -d "$MCP_AGENT_MAIL_DIR" ]; then
        echo -e "${YELLOW}⚠️  Mail server not installed at: $MCP_AGENT_MAIL_DIR${NC}" >&2
        echo -e "${YELLOW}   Agents will not be able to register automatically${NC}" >&2
        return 1
    fi

    # Check if already running
    if [ -f "$PID_FILE" ]; then
        local PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            return 0  # Already running
        else
            rm -f "$PID_FILE"
        fi
    fi

    # Start mail server if available
    if [ -f "$SCRIPT_DIR/start-mail-server.sh" ]; then
        echo -e "${CYAN}Starting agent mail server...${NC}"
        "$SCRIPT_DIR/start-mail-server.sh" >/dev/null 2>&1 || true
    elif [ -f "$PROJECT_ROOT/../AgentCore/flywheel_tools/scripts/adapters/start-mail-server.sh" ]; then
        echo -e "${CYAN}Starting agent mail server...${NC}"
        "$PROJECT_ROOT/../AgentCore/flywheel_tools/scripts/adapters/start-mail-server.sh" >/dev/null 2>&1 || true
    fi

    return 0
}


# Ensure disk space monitor is running
ensure_disk_monitor() {
    local PID_FILE="$PROJECT_ROOT/pids/disk-monitor.pid"

    # Check if already running
    if [ -f "$PID_FILE" ]; then
        local PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            return 0  # Already running
        else
            rm -f "$PID_FILE"
        fi
    fi

    # Start disk monitor if available
    if [ -f "$SCRIPT_DIR/disk-space-monitor.sh" ]; then
        echo -e "${CYAN}Starting disk space monitor...${NC}"
        "$SCRIPT_DIR/disk-space-monitor.sh" start >/dev/null 2>&1 || true
    fi

    return 0
}
# Check if fzf is installed
check_fzf() {
    if ! command -v fzf &> /dev/null; then
        echo -e "${YELLOW}⚠️  fzf is not installed${NC}"
        echo ""
        echo "fzf is required for the visual interface."
        echo ""
        echo -e "${BLUE}Would you like to install it now?${NC}"
        echo ""
        echo -e "  ${GREEN}[Y]${NC} Yes, install fzf (recommended)"
        echo -e "  ${GREEN}[N]${NC} No, use text interface instead"
        echo ""
        read -p "Your choice [Y/n]: " -n 1 install_choice
        echo ""
        echo ""

        case "$install_choice" in
            [Nn])
                echo -e "${BLUE}Using text-based interface...${NC}"
                exec "$SCRIPT_DIR/start-multi-agent-session.sh"
                ;;
            *)
                echo ""
                echo "Please install fzf manually:"
                if [[ "$OSTYPE" == "darwin"* ]]; then
                    echo -e "  ${GREEN}brew install fzf${NC}"
                else
                    echo -e "  ${GREEN}sudo apt install fzf${NC}"
                fi
                echo ""
                echo -e "${BLUE}Falling back to text interface...${NC}"
                sleep 2
                exec "$SCRIPT_DIR/start-multi-agent-session.sh"
                ;;
        esac
    fi
}

# Get list of running tmux sessions (suppressed - not used)
get_running_sessions() {
    tmux list-sessions &>/dev/null && return 0 || return 1
}

# Get list of killed sessions (saved states)
get_killed_sessions() {
    if [ -d "$STATE_DIR" ]; then
        find "$STATE_DIR" -name "*.state" -type f 2>/dev/null | while read -r statefile; do
            basename "$statefile" .state
        done
    fi
}

# Save session state before killing (for resurrection)
save_session_state() {
    local session_name="$1"
    local state_file="$STATE_DIR/${session_name}.state"

    # Save session metadata
    {
        echo "SESSION_NAME=$session_name"
        echo "KILLED_AT=$(date +%s)"
        echo "KILLED_DATE=$(date '+%Y-%m-%d %H:%M:%S')"

        # Save session details
        tmux list-windows -t "$session_name" -F "WINDOW_#{window_index}=#{window_name}" 2>/dev/null || true

        # Save pane information
        tmux list-panes -t "$session_name" -a -F "PANE_#{window_index}_#{pane_index}=#{pane_current_path}|#{pane_current_command}" 2>/dev/null || true

        # Save agent names if available
        tmux list-panes -t "$session_name" -a -F "AGENT_#{window_index}_#{pane_index}=#{@agent_name}" 2>/dev/null | grep -v "AGENT.*=$" || true
    } > "$state_file"

    echo -e "${GREEN}✓ Saved session state: $session_name${NC}" >&2
}

# Resurrect a killed session
resurrect_session() {
    local session_name="$1"
    local state_file="$STATE_DIR/${session_name}.state"

    if [ ! -f "$state_file" ]; then
        echo -e "${RED}❌ No saved state found for: $session_name${NC}"
        return 1
    fi

    # Check if session already exists
    if tmux has-session -t "$session_name" 2>/dev/null; then
        echo -e "${YELLOW}✓ Session already running: $session_name${NC}"
        return 0
    fi

    # Read saved state
    local project_path=""
    if [ -f "$state_file" ]; then
        # Extract project path from first pane's current path
        project_path=$(grep "^PANE_" "$state_file" | head -1 | cut -d'=' -f2 | cut -d'|' -f1)
    fi

    if [ -z "$project_path" ] || [ ! -d "$project_path" ]; then
        project_path="$HOME"
    fi

    # Create basic tmux session in the background
    tmux new-session -d -s "$session_name" -c "$project_path" 2>/dev/null

    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓ Resurrected: $session_name${NC}"
        # Ensure mail server is running for agent registration
        ensure_mail_server
        ensure_disk_monitor
        # Sync beads workflow to the project
        if [ -n "$project_path" ] && [ -d "$project_path" ]; then
            "$SCRIPT_DIR/sync-beads-to-project.sh" "$project_path" 2>/dev/null || true
        fi
        return 0
    else
        echo -e "${RED}❌ Failed to resurrect: $session_name${NC}"
        return 1
    fi
}

# Permanently delete a killed session state and clean up all artifacts
delete_session_state() {
    local session_name="$1"
    local state_file="$STATE_DIR/${session_name}.state"
    local cleanup_count=0

    if [ ! -f "$state_file" ]; then
        echo -e "${YELLOW}⚠️  No state file found for: $session_name${NC}"
        return
    fi

    echo -e "${BLUE}Cleaning up session: $session_name${NC}"

    # 1. Extract agent names from state file and kill their mail monitors
    local agent_names=$(grep "^AGENT_" "$state_file" | cut -d'=' -f2 | sort -u)
    if [ -n "$agent_names" ]; then
        echo -n "  Stopping mail monitors... "
        local monitor_count=0
        while read -r agent_name; do
            [ -z "$agent_name" ] && continue
            # Find and kill mail monitor processes for this agent
            local pids=$(ps aux | grep "[m]onitor-agent-mail.*$agent_name" | awk '{print $2}')
            if [ -n "$pids" ]; then
                echo "$pids" | xargs kill 2>/dev/null || true
                monitor_count=$((monitor_count + $(echo "$pids" | wc -w)))
            fi
        done <<< "$agent_names"
        if [ "$monitor_count" -gt 0 ]; then
            echo "${monitor_count} stopped"
            cleanup_count=$((cleanup_count + monitor_count))
        else
            echo "none found"
        fi
    fi

    # 2. Clean up identity files for this session
    # Identity files follow pattern: {session_name}-{window}-{pane}.identity
    echo -n "  Removing identity files... "
    local identity_count=0
    if [ -d "$PROJECT_ROOT/panes" ]; then
        local identity_files=$(find "$PROJECT_ROOT/panes" -name "${session_name}-*.identity" -type f 2>/dev/null)
        if [ -n "$identity_files" ]; then
            identity_count=$(echo "$identity_files" | wc -l | tr -d ' ')
            # Move to review-for-delete if it exists, otherwise delete
            if [ -d "$PROJECT_ROOT/review-for-delete" ]; then
                echo "$identity_files" | xargs -I {} mv {} "$PROJECT_ROOT/review-for-delete/" 2>/dev/null || true
            else
                echo "$identity_files" | xargs rm -f 2>/dev/null || true
            fi
            echo "${identity_count} removed"
            cleanup_count=$((cleanup_count + identity_count))
        else
            echo "none found"
        fi
    else
        echo "none found"
    fi

    # 3. Clean up PID files for this session
    echo -n "  Removing PID files... "
    local pid_count=0
    if [ -d "$PROJECT_ROOT/pids" ]; then
        local pid_files=$(find "$PROJECT_ROOT/pids" -name "*${session_name}*" -type f 2>/dev/null)
        if [ -n "$pid_files" ]; then
            pid_count=$(echo "$pid_files" | wc -l | tr -d ' ')
            # Move to review-for-delete if it exists, otherwise delete
            if [ -d "$PROJECT_ROOT/review-for-delete" ]; then
                echo "$pid_files" | xargs -I {} mv {} "$PROJECT_ROOT/review-for-delete/" 2>/dev/null || true
            else
                echo "$pid_files" | xargs rm -f 2>/dev/null || true
            fi
            echo "${pid_count} removed"
            cleanup_count=$((cleanup_count + pid_count))
        else
            echo "none found"
        fi
    else
        echo "none found"
    fi

    # 4. Delete the state file itself
    echo -n "  Removing state file... "
    if [ -d "$PROJECT_ROOT/review-for-delete" ]; then
        mv "$state_file" "$PROJECT_ROOT/review-for-delete/" 2>/dev/null && echo "done" || echo "failed"
    else
        rm -f "$state_file" 2>/dev/null && echo "done" || echo "failed"
    fi
    cleanup_count=$((cleanup_count + 1))

    echo ""
    echo -e "${GREEN}✓ Session deleted: $session_name (${cleanup_count} items cleaned)${NC}"
}

# Build session list for fzf with clear sections
build_session_list() {
    local attached_sessions=""
    local running_sessions=""
    local killed_sessions=""

    # Collect sessions by status
    if tmux list-sessions &>/dev/null; then
        while IFS='|' read -r name session_state attached; do
            # Count agents by type in this session
            local agent_summary=""
            local has_identity_files=false

            # Use simple counters instead of associative arrays (bash 3.2 compatible)
            local count_claude=0
            local count_codex=0
            local count_grok=0
            local count_other=0

            # Read identity files for each pane
            while read -r pane_id; do
                [ -z "$pane_id" ] && continue
                local identity_file="panes/${pane_id}.identity"
                if [ -f "$identity_file" ]; then
                    has_identity_files=true
                    local agent_type=$(grep '"type"' "$identity_file" | cut -d'"' -f4)
                    # Normalize type names and count
                    case "$agent_type" in
                        claude-code|claude)
                            count_claude=$((count_claude + 1))
                            ;;
                        codex)
                            count_codex=$((count_codex + 1))
                            ;;
                        grok)
                            count_grok=$((count_grok + 1))
                            ;;
                        *)
                            count_other=$((count_other + 1))
                            ;;
                    esac
                fi
            done < <(tmux list-panes -s -t "$name" -F "#{session_name}-#{window_index}-#{pane_index}" 2>/dev/null)

            # Build agent summary string
            if [ "$has_identity_files" = true ]; then
                # Show breakdown by type
                local parts=""
                [ "$count_claude" -gt 0 ] && parts="${count_claude} Claude"
                [ "$count_codex" -gt 0 ] && parts="${parts:+$parts, }${count_codex} Codex"
                [ "$count_grok" -gt 0 ] && parts="${parts:+$parts, }${count_grok} Grok"
                [ "$count_other" -gt 0 ] && parts="${parts:+$parts, }${count_other} Other"
                agent_summary="$parts"
            fi

            # Fallback: if no identity files, just count panes
            if [ -z "$agent_summary" ]; then
                local pane_count=$(tmux list-panes -s -t "$name" 2>/dev/null | wc -l | tr -d ' ')
                local agent_label="agent"
                [ "$pane_count" != "1" ] && agent_label="agents"
                agent_summary="$pane_count $agent_label"
            fi

            if [ "$attached" = "1" ]; then
                # Attached sessions
                attached_sessions+=$(printf "  🔵  %-28s  │  %-24s │  Active Now|%s|Attached|%s|attached\n" \
                    "$name" "$agent_summary" "$name" "$agent_summary")$'\n'
            else
                # Running but detached
                running_sessions+=$(printf "  🟢  %-28s  │  %-24s │  Background|%s|Running|%s|running\n" \
                    "$name" "$agent_summary" "$name" "$agent_summary")$'\n'
            fi
        done < <(tmux list-sessions -F "#{session_name}|running|#{session_attached}" 2>/dev/null)
    fi

    # Add killed sessions
    while read -r name; do
        if [ -n "$name" ]; then
            local state_file="$STATE_DIR/${name}.state"
            local killed_date=""
            if [ -f "$state_file" ]; then
                killed_date=$(grep "^KILLED_DATE=" "$state_file" | cut -d'=' -f2)
            fi
            killed_sessions+=$(printf "  💀  %-28s  │  Saved      │  %s|%s|Killed|%s|killed\n" \
                "$name" "${killed_date:-Unknown}" "$name" "$killed_date")$'\n'
        fi
    done < <(get_killed_sessions)

    # Build final list with visual section separation
    local final_list=""

    if [ -n "$attached_sessions" ]; then
        final_list+="$attached_sessions"
    fi

    # Add blank line separator between sections
    if [ -n "$attached_sessions" ] && { [ -n "$running_sessions" ] || [ -n "$killed_sessions" ]; }; then
        final_list+=$'\n'
    fi

    if [ -n "$running_sessions" ]; then
        final_list+="$running_sessions"
    fi

    # Add blank line separator before killed sessions
    if [ -n "$running_sessions" ] && [ -n "$killed_sessions" ]; then
        final_list+=$'\n'
    fi

    if [ -n "$killed_sessions" ]; then
        final_list+="$killed_sessions"
    fi

    echo -n "$final_list"
}

# Main visual interface
show_visual_interface() {
    while true; do
        # Clear screen first
        clear

        # Build session list
        local sessions
        sessions=$(build_session_list 2>/dev/null)

        if [ -z "$sessions" ]; then
            echo -e "${YELLOW}No sessions found (running or killed)${NC}"
            echo ""
            echo -e "${GREEN}[S]${NC} Smart Start (analyze queue, auto-spawn)"
            echo -e "${GREEN}[N]${NC} Create new session"
            echo -e "${GREEN}[Q]${NC} Quit"
            echo ""
            read -p "Your choice: " -n 1 choice
            echo ""

            case "$choice" in
                [Ss])
                    smart_start
                    ;;
                [Nn])
                    create_new_session
                    ;;
                [Qq])
                    exit 0
                    ;;
            esac
            continue
        fi

        # Show session list with fzf
        local selected=$(echo "$sessions" | fzf \
            --ansi \
            --multi \
            --expect=ctrl-n,ctrl-s \
            --header="
╔══════════════════════════════════════════════════════════════════════╗
║                  🎡  Agent Flywheel - Session Manager                ║
╠══════════════════════════════════════════════════════════════════════╣
║  🔵 Attached (viewing now)  │  🟢 Running (background)  │  💀 Saved  ║
╠══════════════════════════════════════════════════════════════════════╣
║   ↑↓  Move    │   Tab  Select Multiple    │   Enter  Actions       ║
║   Esc/Ctrl-C  Quit   │  Ctrl-N  New   │  Ctrl-S  Smart Start      ║
╚══════════════════════════════════════════════════════════════════════╝
" \
            --header-lines=0 \
            --preview='
# Check if line is blank or just whitespace
if [ -z "$(echo {} | tr -d "[:space:]")" ]; then
    echo ""
    echo "  ═══════════════════════════════════"
    echo "    Visual Separator"
    echo "  ═══════════════════════════════════"
    echo ""
    echo "  This is just spacing between"
    echo "  session groups."
    echo ""
    echo "  Select actual sessions instead."
    echo ""
    exit 0
fi

session_name=$(echo {} | cut -d"|" -f2)
session_status=$(echo {} | cut -d"|" -f3)

echo ""
echo "╔════════════════════════════════════╗"
echo "║      SESSION INFORMATION           ║"
echo "╚════════════════════════════════════╝"
echo ""
echo "  Name:     $session_name"
echo "  Status:   $session_status"
echo ""
if [ "$session_status" = "Attached" ]; then
    echo "  📍 You are currently viewing"
    echo "     this session in another"
    echo "     window/tab."
    echo ""
    echo "  Actions:"
    echo "  • Detach: Ctrl+b then d"
elif [ "$session_status" = "Running" ]; then
    echo "  ⚡ Agents working in background"
    echo ""
    echo "  Actions:"
    echo "  • Press Enter → A to attach"
    echo "  • Press Enter → K to kill"
elif [ "$session_status" = "Killed" ]; then
    echo "  💾 Session saved to disk"
    echo ""
    echo "  Actions:"
    echo "  • Press Enter → R to resurrect"
    echo "  • Press Enter → D to delete"
fi
echo ""
' \
            --preview-window=right:45%:wrap:border-rounded \
            --bind='ctrl-a:select-all,ctrl-d:deselect-all' \
            --prompt="Select ❯ " \
            --pointer="▶ " \
            --marker="✓ " \
            --delimiter="|" \
            --with-nth=1 \
            --layout=reverse \
            --height=95% \
            --border=rounded \
            --border-label="╣ Select Sessions ╠" \
            --no-info \
            --color='fg:#e0e0e0,bg:#0a0a0a,hl:#00d7ff' \
            --color='fg+:#ffffff,bg+:#1a1a1a,hl+:#00ffff' \
            --color='info:#00d7ff,prompt:#00d7ff,pointer:#ff00ff' \
            --color='marker:#00ff00,spinner:#ff00ff,header:#00d7ff' \
            --color='border:#555555,label:#00d7ff,preview-border:#555555')

        # Extract the key pressed (first line with --expect)
        local key_pressed=$(echo "$selected" | head -1)
        local sessions_selected=$(echo "$selected" | tail -n +2)

        # Check if user pressed Ctrl+N to create new session
        if [ "$key_pressed" = "ctrl-n" ]; then
            create_new_session
            continue
        fi

        # Check if user pressed Ctrl+S for Smart Start
        if [ "$key_pressed" = "ctrl-s" ]; then
            smart_start
            continue
        fi

        # Use sessions_selected instead of selected from now on
        selected="$sessions_selected"

        if [ -z "$selected" ]; then
            # User cancelled (Ctrl-C or q)
            echo ""
            echo -e "${YELLOW}What would you like to do?${NC}"
            echo -e "${GREEN}[S]${NC} Smart Start (analyze queue, auto-spawn)"
            echo -e "${GREEN}[N]${NC} Create new session"
            echo -e "${GREEN}[Q]${NC} Quit"
            echo ""
            read -p "Your choice: " -n 1 choice
            echo ""

            case "$choice" in
                [Ss])
                    smart_start
                    ;;
                [Nn])
                    create_new_session
                    ;;
                *)
                    exit 0
                    ;;
            esac
            continue
        fi

        # Filter out blank lines and whitespace-only lines
        selected=$(echo "$selected" | grep -v '^[[:space:]]*$')

        # If no valid sessions after filtering, go back to menu
        if [ -z "$selected" ]; then
            continue
        fi

        # Process selected sessions
        show_action_menu "$selected"
    done
}

# Show action menu based on selection
show_action_menu() {
    local selected="$1"
    local count=$(echo "$selected" | wc -l)

    # Determine session types
    local has_running=false
    local has_killed=false

    echo "$selected" | while IFS='|' read -r display name1 name2 status rest; do
        local session_type=$(echo "$rest" | rev | cut -d'|' -f1 | rev)
        if [ "$session_type" = "running" ]; then
            has_running=true
        elif [ "$session_type" = "killed" ]; then
            has_killed=true
        fi
    done

    # Count session types by checking the last field
    local has_running=0
    local has_killed=0

    while IFS='|' read -r line; do
        # Get the last field (session type)
        local type=$(echo "$line" | rev | cut -d'|' -f1 | rev | xargs)
        if [ "$type" = "attached" ] || [ "$type" = "running" ]; then
            has_running=$((has_running + 1))
        elif [ "$type" = "killed" ]; then
            has_killed=$((has_killed + 1))
        fi
    done <<< "$selected"

    echo ""
    echo -e "${BOLD}Selected $count session(s):${NC}"
    echo ""

    # Show which sessions are selected
    echo "$selected" | while IFS='|' read -r display session_name status rest; do
        local session_type=$(echo "$rest" | rev | cut -d'|' -f1 | rev)
        if [ "$session_type" = "attached" ] || [ "$session_type" = "running" ]; then
            echo -e "  ${GREEN}▸${NC} ${CYAN}$session_name${NC} (Running)"
        elif [ "$session_type" = "killed" ]; then
            echo -e "  ${GRAY}▸${NC} ${CYAN}$session_name${NC} (Saved)"
        fi
    done
    echo ""

    # Build action menu based on what's actually selected
    local actions=""

    # Running/Attached sessions
    if [ "$has_running" != "0" ] && [ "$has_killed" = "0" ]; then
        # ONLY running sessions selected
        echo -e "${GREEN}✓ Running sessions selected${NC}"
        echo ""
        actions="${actions}[A] Attach to session(s)\n"
        actions="${actions}[K] Kill session(s) (saves them)\n"
    # Killed sessions
    elif [ "$has_killed" != "0" ] && [ "$has_running" = "0" ]; then
        # ONLY killed sessions selected
        echo -e "${GRAY}💀 Saved sessions selected${NC}"
        echo ""
        actions="${actions}[R] Resurrect session(s)\n"
        actions="${actions}[D] Permanently delete session(s)\n"
    # Mixed selection
    elif [ "$has_running" != "0" ] && [ "$has_killed" != "0" ]; then
        # Mixed - both running and killed
        echo -e "${YELLOW}⚠️  Mixed selection (running + saved)${NC}"
        echo ""
        actions="${actions}[A] Attach to running session(s)\n"
        actions="${actions}[K] Kill running session(s)\n"
        actions="${actions}[R] Resurrect saved session(s)\n"
        actions="${actions}[D] Delete saved session(s)\n"
    fi

    actions="${actions}[S] Smart Start (analyze queue, auto-spawn)\n"
    actions="${actions}[N] Create new session\n"
    actions="${actions}[C] Cancel"

    echo -e "$actions"
    echo ""
    read -p "Your choice: " -n 1 action
    echo ""

    case "$action" in
        [Aa])
            attach_sessions "$selected"
            ;;
        [Kk])
            kill_sessions "$selected"
            ;;
        [Rr])
            resurrect_sessions "$selected"
            ;;
        [Dd])
            delete_sessions "$selected"
            ;;
        [Ss])
            smart_start
            ;;
        [Nn])
            create_new_session
            ;;
        *)
            return
            ;;
    esac
}

# Attach to selected sessions
attach_sessions() {
    local selected="$1"

    # Check for killed sessions that need resurrection first
    local killed_sessions=$(echo "$selected" | grep "|killed$" || true)
    local running_sessions=$(echo "$selected" | grep -E "\|running$|\|attached$" || true)

    # Resurrect any killed sessions first
    if [ -n "$killed_sessions" ]; then
        echo ""
        echo -e "${YELLOW}⚙️  Resurrecting saved sessions first...${NC}"
        echo ""

        echo "$killed_sessions" | while IFS='|' read -r display session_name rest; do
            session_name=$(echo "$session_name" | xargs)
            echo -e "  ${CYAN}→${NC} Resurrecting: $session_name"
            resurrect_session "$session_name"
        done

        echo ""
        echo -e "${GREEN}✓ All saved sessions resurrected${NC}"
        sleep 1
    fi

    # Now collect all sessions to attach (running + newly resurrected)
    local all_sessions=""
    echo "$selected" | while IFS='|' read -r display session_name rest; do
        session_name=$(echo "$session_name" | xargs)
        # Check if session is now running in tmux
        if tmux has-session -t "$session_name" 2>/dev/null; then
            echo "$session_name"
        fi
    done > /tmp/attach-sessions-list.$$

    all_sessions=$(cat /tmp/attach-sessions-list.$$ 2>/dev/null || echo "")
    rm -f /tmp/attach-sessions-list.$$

    if [ -z "$all_sessions" ]; then
        echo -e "${RED}No sessions available to attach${NC}"
        sleep 2
        return
    fi

    # Ensure mail server is running for agent registration
    ensure_mail_server
        ensure_disk_monitor

    # Sync beads workflow to each session's project
    echo ""
    echo -e "${CYAN}Syncing beads workflow to projects...${NC}"
    echo "$all_sessions" | while read -r sess; do
        sess=$(echo "$sess" | xargs)
        local proj_path=$(tmux display-message -t "$sess" -p '#{pane_current_path}' 2>/dev/null)
        if [ -n "$proj_path" ] && [ -d "$proj_path" ]; then
            "$SCRIPT_DIR/sync-beads-to-project.sh" "$proj_path" 2>/dev/null || true
        fi
    done

    local count=$(echo "$all_sessions" | wc -l | tr -d ' ')

    if [ "$count" -eq 1 ]; then
        # Single session - attach directly
        local session_name=$(echo "$all_sessions" | head -1 | xargs)
        echo ""
        echo -e "${GREEN}✓ Session ready: $session_name${NC}"
        echo ""

        # Check if we're in an interactive terminal
        if [ -t 0 ] && [ -t 1 ]; then
            echo -e "${CYAN}Attaching now...${NC}"
            sleep 1
            exec tmux attach -t "$session_name"
        else
            # Non-interactive - print manual attach command
            echo -e "${YELLOW}Run this command to attach:${NC}"
            echo -e "  ${CYAN}tmux attach -t $session_name${NC}"
            echo ""
            exit 0
        fi
    else
        # Multiple sessions
        echo ""
        echo -e "${GREEN}✓ All sessions ready ($count total)${NC}"
        echo ""

        if [[ "${TERM_PROGRAM:-}" == "iTerm.app" ]] && [ -t 0 ]; then
            echo -e "${CYAN}Opening in new tabs...${NC}"
            echo "$all_sessions" | while read -r session_name; do
                session_name=$(echo "$session_name" | xargs)
                osascript 2>/dev/null <<EOF
tell application "iTerm"
    tell current window
        create tab with default profile command "tmux attach -t $session_name"
    end tell
end tell
EOF
            done
            echo ""
            echo -e "${GREEN}✓ Opened in iTerm tabs${NC}"
            sleep 2
        else
            # Non-iTerm or non-interactive - show manual commands
            echo -e "${YELLOW}To attach to sessions, run:${NC}"
            echo ""
            echo "$all_sessions" | while read -r session_name; do
                session_name=$(echo "$session_name" | xargs)
                echo -e "  ${CYAN}tmux attach -t $session_name${NC}"
            done
            echo ""
            echo -e "${GRAY}Or use Ctrl-C to cancel and attach manually${NC}"
            sleep 3
        fi
    fi
}

# Kill selected sessions
kill_sessions() {
    local selected="$1"
    local running_sessions=$(echo "$selected" | grep -E "\|running$|\|attached$" || true)
    local killed_sessions=$(echo "$selected" | grep "|killed$" || true)

    # Warn if some sessions are already killed
    if [ -n "$killed_sessions" ]; then
        local killed_count=$(echo "$killed_sessions" | wc -l | tr -d ' ')
        echo ""
        echo -e "${YELLOW}⚠️  Note: $killed_count session(s) already saved (will be skipped)${NC}"
        sleep 1
    fi

    if [ -z "$running_sessions" ]; then
        echo ""
        echo -e "${RED}No running sessions to kill${NC}"
        sleep 2
        return
    fi

    echo ""
    echo "$running_sessions" | while IFS='|' read -r display session_name rest; do
        session_name=$(echo "$session_name" | xargs)
        echo -e "  ${YELLOW}→${NC} Saving and killing: $session_name"
        save_session_state "$session_name"
        tmux kill-session -t "$session_name" 2>/dev/null || true
    done

    echo ""
    echo -e "${GREEN}✓ Running sessions killed and saved${NC}"
    sleep 2
}

# Resurrect selected sessions
resurrect_sessions() {
    local selected="$1"
    local killed_sessions=$(echo "$selected" | grep "|killed$" || true)
    local running_sessions=$(echo "$selected" | grep -E "\|running$|\|attached$" || true)

    # Warn if some sessions are already running
    if [ -n "$running_sessions" ]; then
        local running_count=$(echo "$running_sessions" | wc -l | tr -d ' ')
        echo ""
        echo -e "${YELLOW}⚠️  Note: $running_count session(s) already running (will be skipped)${NC}"
        sleep 1
    fi

    if [ -z "$killed_sessions" ]; then
        echo ""
        echo -e "${RED}No saved sessions to resurrect${NC}"
        sleep 2
        return
    fi

    local count=$(echo "$killed_sessions" | wc -l | tr -d ' ')

    if [ "$count" -eq 1 ]; then
        # Single session - resurrect and attach
        local session_name=$(echo "$killed_sessions" | head -1 | cut -d'|' -f2)
        session_name=$(echo "$session_name" | xargs)
        resurrect_session "$session_name"
    else
        # Multiple sessions - resurrect all
        echo ""
        echo -e "${GREEN}Resurrecting $count sessions...${NC}"
        echo ""

        echo "$killed_sessions" | while IFS='|' read -r display session_name rest; do
            session_name=$(echo "$session_name" | xargs)
            echo -e "  ${CYAN}→${NC} Resurrecting: $session_name"
            resurrect_session "$session_name"
        done

        echo ""
        echo -e "${GREEN}✓ All sessions resurrected${NC}"
        sleep 2
    fi
}

# Delete selected sessions permanently
delete_sessions() {
    local selected="$1"
    local killed_sessions=$(echo "$selected" | grep "|killed$" || true)
    local running_sessions=$(echo "$selected" | grep -E "\|running$|\|attached$" || true)

    # Warn if some sessions are still running
    if [ -n "$running_sessions" ]; then
        local running_count=$(echo "$running_sessions" | wc -l | tr -d ' ')
        echo ""
        echo -e "${YELLOW}⚠️  Note: $running_count session(s) still running (cannot delete)${NC}"
        echo -e "${YELLOW}    Kill them first to save their state${NC}"
        sleep 1
    fi

    if [ -z "$killed_sessions" ]; then
        echo ""
        echo -e "${RED}No saved sessions to delete${NC}"
        sleep 2
        return
    fi

    echo ""
    echo -e "${RED}⚠️  This will permanently delete the session state files!${NC}"
    read -p "Are you sure? [y/N]: " -n 1 confirm
    echo ""

    if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
        return
    fi

    echo "$killed_sessions" | while IFS='|' read -r display session_name rest; do
        session_name=$(echo "$session_name" | xargs)
        delete_session_state "$session_name"
    done

    echo ""
    echo -e "${GREEN}✓ Saved sessions permanently deleted${NC}"
    sleep 2
}

# Smart Start - analyze bead queue and spawn typed agents running agent-runner.sh
smart_start() {
    clear
    echo ""
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║              Smart Start - Queue-Driven Spawning              ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    # Check if plan-to-agents.sh exists
    if [ ! -f "$SCRIPT_DIR/plan-to-agents.sh" ]; then
        echo -e "${RED}Error: plan-to-agents.sh not found${NC}"
        sleep 2
        return
    fi

    # Step 1: Select project folder
    echo -e "${BLUE}Step 1: Select project folder${NC}"
    echo ""
    local project_path=$("$SCRIPT_DIR/file-picker.sh" folder)

    if [ -z "$project_path" ]; then
        echo -e "${YELLOW}No folder selected, cancelled${NC}"
        sleep 1
        return
    fi

    if [ ! -d "$project_path" ]; then
        echo -e "${RED}Error: Not a valid directory${NC}"
        sleep 2
        return
    fi

    echo -e "${GREEN}Selected: $project_path${NC}"
    echo ""

    # Sync beads workflow scripts to the target project
    echo -e "${CYAN}Syncing beads workflow...${NC}"
    "$SCRIPT_DIR/sync-beads-to-project.sh" "$project_path" 2>/dev/null || true
    echo ""

    # Ensure mail server is running for agent registration
    ensure_mail_server
        ensure_disk_monitor

    # Step 2: Analyze bead queue for the selected project
    echo -e "${CYAN}Analyzing bead queue...${NC}"
    echo ""
    "$SCRIPT_DIR/plan-to-agents.sh" --project "$project_path"

    echo ""
    echo -e "${YELLOW}Options:${NC}"
    echo -e "  ${GREEN}[Y]${NC} Spawn recommended agents (runs agent-runner.sh in each pane)"
    echo -e "  ${GREEN}[C]${NC} Customize (enter manual session creation)"
    echo -e "  ${GREEN}[Q]${NC} Cancel"
    echo ""
    read -p "Your choice: " -n 1 smart_choice
    echo ""

    case "$smart_choice" in
        [Yy])
            echo ""
            local default_name
            default_name=$(basename "$project_path")
            echo -en "${YELLOW}Session name (press Enter for '$default_name'):${NC} "
            read smart_session_name || smart_session_name=""
            smart_session_name=${smart_session_name:-$default_name}

            local spawn_args="--auto --project $project_path"
            if [ -n "$smart_session_name" ]; then
                spawn_args="$spawn_args --session $smart_session_name"
            fi

            echo ""
            echo -e "${GREEN}Spawning agents...${NC}"
            "$SCRIPT_DIR/plan-to-agents.sh" $spawn_args

            echo ""
            echo -e "${GREEN}Done! Agents are running agent-runner.sh in each pane.${NC}"
            sleep 3
            ;;
        [Cc])
            create_new_session "$project_path"
            ;;
        *)
            return
            ;;
    esac
}

# Create a new session - fully integrated visual workflow
create_new_session() {
    local project_path="$1"  # Optional: pre-selected project path
    clear
    echo ""
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║              Create New Multi-Agent Session                   ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    # Step 1: Select project folder (skip if already provided)
    if [ -z "$project_path" ]; then
        echo -e "${BLUE}Step 1/4: Select project folder${NC}"
        echo ""
        project_path=$("$SCRIPT_DIR/file-picker.sh" folder)
    else
        echo -e "${BLUE}Step 1/4: Project folder${NC}"
        echo -e "${GREEN}✓ Using: $project_path${NC}"
        echo ""
    fi

    if [ -z "$project_path" ]; then
        echo -e "${YELLOW}No folder selected, cancelled${NC}"
        sleep 1
        return
    fi

    if [ ! -d "$project_path" ]; then
        echo -e "${RED}Error: Not a valid directory${NC}"
        sleep 2
        return
    fi

    echo -e "${GREEN}✓ Selected: $project_path${NC}"
    echo ""

    # Sync beads workflow scripts to the target project
    echo -e "${CYAN}Syncing beads workflow...${NC}"
    "$SCRIPT_DIR/sync-beads-to-project.sh" "$project_path" 2>/dev/null || true
    echo ""

    # Step 2: Session name
    echo -e "${BLUE}Step 2/4: Session name${NC}"
    local session_name=""
    while true; do
        echo -en "${YELLOW}Session name (or press Enter for '$(basename "$project_path")'):${NC} "
        read session_name || session_name=""
        session_name=${session_name:-$(basename "$project_path")}

        # Sanitize for tmux
        local session_safe=$(echo "$session_name" | tr -cs 'A-Za-z0-9_-' '_' | tr '[:upper:]' '[:lower:]' | sed 's/^_*//;s/_*$//')

        if [ -z "$session_safe" ]; then
            echo -e "${RED}Error: Invalid session name${NC}"
            continue
        fi

        # Check if session already exists
        if tmux has-session -t "$session_safe" 2>/dev/null; then
            echo -e "${YELLOW}Session '$session_safe' already exists${NC}"
            echo -en "${YELLOW}Choose a different name? [Y/n]:${NC} "
            read try_again || try_again=""
            try_again=${try_again:-Y}
            if [[ "$try_again" =~ ^[Yy]$ ]] || [ "$try_again" = "" ]; then
                continue
            else
                return
            fi
        fi

        if [ "$session_safe" != "$session_name" ]; then
            echo -e "${YELLOW}Note: Normalized to '$session_safe'${NC}"
        fi

        break
    done
    echo -e "${GREEN}✓ Session name: $session_safe${NC}"
    echo ""

    # Step 3: Number of agents
    echo -e "${BLUE}Step 3/4: Agent configuration${NC}"
    echo -e "${YELLOW}💡 Tip: Start with 2 Claude agents if you're not sure${NC}"
    echo ""

    local claude_count=""
    while true; do
        echo -en "${YELLOW}Number of Claude agents (press Enter for 2):${NC} "
        read claude_count || claude_count=""
        claude_count=${claude_count:-2}

        if [[ "$claude_count" =~ ^[0-9]+$ ]]; then
            break
        else
            echo -e "${RED}Error: Must be a number${NC}"
        fi
    done

    local chatgpt_count=""
    while true; do
        echo -en "${YELLOW}Number of ChatGPT agents via OAuth (press Enter for 0):${NC} "
        read chatgpt_count || chatgpt_count=""
        chatgpt_count=${chatgpt_count:-0}

        if [[ "$chatgpt_count" =~ ^[0-9]+$ ]]; then
            break
        else
            echo -e "${RED}Error: Must be a number${NC}"
        fi
    done

    local codex_count=""
    while true; do
        echo -en "${YELLOW}Number of Codex agents (press Enter for 0):${NC} "
        read codex_count || codex_count=""
        codex_count=${codex_count:-0}

        if [[ "$codex_count" =~ ^[0-9]+$ ]]; then
            break
        else
            echo -e "${RED}Error: Must be a number${NC}"
        fi
    done

    local deepseek_count=""
    while true; do
        echo -en "${YELLOW}Number of DeepSeek agents (press Enter for 0):${NC} "
        read deepseek_count || deepseek_count=""
        deepseek_count=${deepseek_count:-0}

        if [[ "$deepseek_count" =~ ^[0-9]+$ ]]; then
            break
        else
            echo -e "${RED}Error: Must be a number${NC}"
        fi
    done

    local grok_count=""
    while true; do
        echo -en "${YELLOW}Number of Grok agents (press Enter for 0):${NC} "
        read grok_count || grok_count=""
        grok_count=${grok_count:-0}

        if [[ "$grok_count" =~ ^[0-9]+$ ]]; then
            break
        else
            echo -e "${RED}Error: Must be a number${NC}"
        fi
    done

    local total_agents=$((claude_count + chatgpt_count + codex_count + deepseek_count + grok_count))

    if [ "$total_agents" -eq 0 ]; then
        echo -e "${RED}Error: Must have at least one agent${NC}"
        sleep 2
        return
    fi

    echo -e "${GREEN}✓ Agents: $claude_count Claude + $chatgpt_count ChatGPT + $codex_count Codex + $deepseek_count DeepSeek + $grok_count Grok = $total_agents total${NC}"
    echo ""

    # Authentication setup for non-Claude agents
    local need_auth=0

    if [ "$chatgpt_count" -gt 0 ]; then
        echo -e "${YELLOW}ChatGPT agents require authentication${NC}"
        echo -en "${YELLOW}Run ChatGPT setup now? [Y/n]:${NC} "
        read setup_chatgpt || setup_chatgpt=""
        setup_chatgpt=${setup_chatgpt:-Y}

        if [[ "$setup_chatgpt" =~ ^[Yy]$ ]]; then
            echo -e "${BLUE}Running ChatGPT setup...${NC}"
            "$SCRIPT_DIR/setup-chatgpt.sh"
            echo ""
        fi
        need_auth=1
    fi

    if [ "$deepseek_count" -gt 0 ]; then
        if [ -z "${DEEPSEEK_API_KEY:-}" ]; then
            echo -e "${YELLOW}DeepSeek agents require API key authentication${NC}"
            echo -en "${YELLOW}Run DeepSeek setup now? [Y/n]:${NC} "
            read setup_deepseek || setup_deepseek=""
            setup_deepseek=${setup_deepseek:-Y}

            if [[ "$setup_deepseek" =~ ^[Yy]$ ]]; then
                echo -e "${BLUE}Running DeepSeek setup...${NC}"
                "$SCRIPT_DIR/setup-deepseek.sh"
                # Source the shell RC to get the new API key
                if [ -f "$HOME/.zshrc" ]; then
                    source "$HOME/.zshrc"
                elif [ -f "$HOME/.bashrc" ]; then
                    source "$HOME/.bashrc"
                fi
                echo ""
            fi
            need_auth=1
        else
            echo -e "${GREEN}✓ DeepSeek API key already configured${NC}"
        fi
    fi

    if [ "$grok_count" -gt 0 ]; then
        if [ -z "${XAI_API_KEY:-}" ]; then
            echo -e "${YELLOW}Grok agents require xAI API key authentication${NC}"
            echo -en "${YELLOW}Run Grok setup now? [Y/n]:${NC} "
            read setup_grok || setup_grok=""
            setup_grok=${setup_grok:-Y}

            if [[ "$setup_grok" =~ ^[Yy]$ ]]; then
                echo -e "${BLUE}Running Grok setup...${NC}"
                "$SCRIPT_DIR/setup-grok.sh"
                # Source the shell RC to get the new API key
                if [ -f "$HOME/.zshrc" ]; then
                    source "$HOME/.zshrc"
                elif [ -f "$HOME/.bashrc" ]; then
                    source "$HOME/.bashrc"
                fi
                echo ""
            fi
            need_auth=1
        else
            echo -e "${GREEN}✓ Grok API key already configured${NC}"
        fi
    fi

    if [ "$codex_count" -gt 0 ]; then
        if [ -z "${OPENAI_API_KEY:-}" ]; then
            echo -e "${YELLOW}Codex agents require OpenAI API key${NC}"
            echo -en "${YELLOW}Run OpenAI setup now? [Y/n]:${NC} "
            read setup_openai || setup_openai=""
            setup_openai=${setup_openai:-Y}

            if [[ "$setup_openai" =~ ^[Yy]$ ]]; then
                echo -e "${BLUE}Running OpenAI setup...${NC}"
                "$SCRIPT_DIR/setup-openai-key.sh"
                # Source the shell RC to get the new API key
                if [ -f "$HOME/.zshrc" ]; then
                    source "$HOME/.zshrc"
                elif [ -f "$HOME/.bashrc" ]; then
                    source "$HOME/.bashrc"
                fi
                echo ""
            fi
            need_auth=1
        else
            echo -e "${GREEN}✓ OpenAI API key already configured${NC}"
        fi
    fi

    if [ "$need_auth" -eq 1 ]; then
        echo ""
    fi

    # Step 4: Shared task list
    echo -e "${BLUE}Step 4/4: Shared task list${NC}"
    echo -e "${YELLOW}Shared task lists allow all agents to collaborate on the same tasks${NC}"
    echo ""

    local enable_shared=""
    echo -en "${YELLOW}Enable shared task list? [y/N]:${NC} "
    read enable_shared || enable_shared=""
    enable_shared=${enable_shared:-N}

    local task_list_id=""
    if [[ "$enable_shared" =~ ^[Yy]$ ]]; then
        echo -en "${YELLOW}Task list ID (press Enter for '${session_safe}-tasks'):${NC} "
        read task_list_id || task_list_id=""
        task_list_id=${task_list_id:-"${session_safe}-tasks"}
        echo -e "${GREEN}✓ Shared task list: $task_list_id${NC}"
    else
        echo -e "${BLUE}Each agent will have its own task list${NC}"
    fi
    echo ""

    # Confirm and create
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║                    Ready to Create                            ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "  ${BLUE}Session:${NC}     $session_safe"
    echo -e "  ${BLUE}Project:${NC}     $project_path"
    echo -e "  ${BLUE}Agents:${NC}      $claude_count Claude + $chatgpt_count ChatGPT + $codex_count Codex + $deepseek_count DeepSeek + $grok_count Grok"
    if [[ "$enable_shared" =~ ^[Yy]$ ]]; then
        echo -e "  ${BLUE}Tasks:${NC}       Shared ($task_list_id)"
    else
        echo -e "  ${BLUE}Tasks:${NC}       Individual per agent"
    fi
    echo ""
    echo -en "${YELLOW}Create session? [Y/n]:${NC} "
    read confirm || confirm=""
    confirm=${confirm:-Y}

    if ! [[ "$confirm" =~ ^[Yy]$ ]] && [ "$confirm" != "" ]; then
        echo -e "${YELLOW}Cancelled${NC}"
        sleep 1
        return
    fi

    # Export all parameters and call session creation script
    export SELECTED_PROJECT_PATH="$project_path"
    export SKIP_EXISTING_SESSIONS_CHECK=1
    export PRESET_SESSION_NAME="$session_safe"
    export PRESET_CLAUDE_COUNT="$claude_count"
    export PRESET_CHATGPT_COUNT="$chatgpt_count"
    export PRESET_CODEX_COUNT="$codex_count"
    export PRESET_DEEPSEEK_COUNT="$deepseek_count"
    export PRESET_GROK_COUNT="$grok_count"
    export PRESET_ENABLE_SHARED_TASKS="$enable_shared"
    export PRESET_TASK_LIST_ID="$task_list_id"

    echo ""
    echo -e "${GREEN}🚀 Creating session...${NC}"
    exec "$SCRIPT_DIR/start-multi-agent-session.sh"
}

# Main entry point
main() {
    check_fzf
    show_visual_interface
}

main "$@"
