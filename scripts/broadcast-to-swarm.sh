#!/usr/bin/env bash
# broadcast-to-swarm.sh - Broadcast messages to agent swarms
#
# Usage:
#   ./scripts/broadcast-to-swarm.sh <group> <subject> <message> [options]
#
# Groups:
#   @all                - All agents in current project
#   @active             - All agents with active tmux panes
#   @swarm:<name>       - All agents in specific swarm session
#   @type:<type>        - All agents of specific type (backend, frontend, etc.)
#   @coordinators       - Special group for coordinators
#
# Options:
#   --importance <level>    Message importance: normal, urgent (default: normal)
#   --type <type>           Message type: URGENT, FYI, BLOCKER, HANDOFF, etc.
#   --mail-only             Skip tmux notifications, use mail only
#   --tmux-only             Skip mail delivery, use tmux only
#   --dry-run               Show what would be sent without sending
#
# Part of: bd-1no - Broadcast Service (Phase 1 NTM)
# Extension: Custom Mail Types (flywheel optimization)

set -euo pipefail

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Project root and paths
PIDS_DIR="$PROJECT_ROOT/pids"
AGENT_REGISTRY="$PROJECT_ROOT/scripts/agent-registry.sh"
MAIL_HELPER="$PROJECT_ROOT/scripts/agent-mail-helper.sh"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m'

#######################################
# Get importance level for message type
# Arguments:
#   $1 - Message type (URGENT, FYI, etc.)
# Returns: Importance level (urgent or normal)
#######################################
get_message_importance() {
    local msg_type="$1"

    case "$msg_type" in
        URGENT|BLOCKER)
            echo "urgent"
            ;;
        FYI|HANDOFF|UPDATE|QUESTION|COMPLETED|*)
            echo "normal"
            ;;
    esac
}

#######################################
# Print colored message
#######################################
print_msg() {
    local color="${!1}"
    local msg="$2"
    echo -e "${color}${msg}${NC}" >&2
}

#######################################
# Print usage
#######################################
usage() {
    cat <<EOF
Usage: $(basename "$0") <group> <subject> <message> [options]

Broadcast messages to groups of agents via tmux + agent mail.

Group Types:
  @all                    All agents in current project
  @active                 All agents with active tmux panes
  @swarm:<name>           All agents in specific swarm session
  @type:<type>            All agents of specific type (backend, frontend, etc.)
  @coordinators           Special group for coordinators

Options:
  --importance <level>    Message importance: normal, urgent (default: normal)
  --type <type>           Message type: URGENT, FYI, BLOCKER, HANDOFF, etc.
  --mail-only             Skip tmux notifications, use mail only
  --tmux-only             Skip mail delivery, use tmux only
  --dry-run               Show what would be sent without sending
  --help                  Show this help message

Message Types (bd-1no extension):
  URGENT                  High priority, requires immediate attention
  FYI                     Informational, no action required
  BLOCKER                 Task is blocked, needs resolution
  HANDOFF                 Task handoff to another agent
  UPDATE                  Progress update
  QUESTION                Question requiring response
  COMPLETED               Task completion notification

Delivery Methods:
  - Tmux: Immediate notification via send-keys (non-blocking)
  - Mail: Persistent message via agent mail system

Examples:
  # Broadcast to all agents
  $(basename "$0") @all "System maintenance" "Pausing work in 10 minutes"

  # Broadcast to specific swarm
  $(basename "$0") @swarm:phase3 "Status check" "Report your progress"

  # Broadcast to agents of specific type
  $(basename "$0") @type:backend "API change" "New auth endpoint available"

  # Broadcast to coordinators (urgent)
  $(basename "$0") @coordinators "Queue alert" "Critical threshold reached" --importance urgent

  # Use message type
  $(basename "$0") @all "Deployment" "Starting deployment" --type URGENT

  # Mail only (no tmux interruption)
  $(basename "$0") @swarm:phase3 "FYI: Docs updated" "Check CLAUDE.md" --mail-only

  # Dry run (test without sending)
  $(basename "$0") @all "Test" "Test message" --dry-run

Requirements:
  - Agent mail system (./scripts/agent-mail-helper.sh)
  - Tmux session(s) with registered agents
  - For @type: agent-registry.sh with type definitions

EOF
}

#######################################
# Resolve group to list of agents
# Arguments:
#   $1 - Group specifier (@all, @active, @swarm:<name>, @type:<type>, @coordinators)
# Returns: Space-separated list of agent names
#######################################
resolve_group() {
    local group="$1"
    local agents=""

    case "$group" in
        @all)
            # All agents with identity files
            agents=$(find "$PROJECT_ROOT/.beads/panes" -name "*.identity" -type f 2>/dev/null | while IFS= read -r identity_file; do
                jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null
            done | sort -u | tr '\n' ' ' | xargs)
            ;;

        @active)
            # All agents with active tmux panes
            agents=$(find "$PROJECT_ROOT/.beads/panes" -name "*.identity" -type f 2>/dev/null | while IFS= read -r identity_file; do
                local pane=$(jq -r '.pane // empty' "$identity_file" 2>/dev/null)
                local agent_name=$(jq -r '.agent_mail_name // empty' "$identity_file" 2>/dev/null)

                if [ -n "$pane" ] && [ -n "$agent_name" ]; then
                    # Check if pane still exists
                    if tmux list-panes -a -F "#{pane_id}" 2>/dev/null | grep -q "^$pane$"; then
                        echo "$agent_name"
                    fi
                fi
            done | sort -u | tr '\n' ' ' | xargs)
            ;;

        @swarm:*)
            # Agents in specific swarm session
            local session_name="${group#@swarm:}"
            local state_file="$PIDS_DIR/swarm-${session_name}.state"

            if [ ! -f "$state_file" ]; then
                print_msg YELLOW "Warning: Swarm session '$session_name' not found"
                echo ""
                return
            fi

            agents=$(jq -r '.agents[].name' "$state_file" 2>/dev/null | tr '\n' ' ' | xargs)
            ;;

        @type:*)
            # Agents of specific type
            local agent_type="${group#@type:}"
            agents=$(find "$PROJECT_ROOT/.agent-profiles/instances" -name "*.json" -type f 2>/dev/null | while IFS= read -r instance_file; do
                local type=$(jq -r '.type // "general"' "$instance_file" 2>/dev/null)
                if [ "$type" = "$agent_type" ]; then
                    local agent_name=$(basename "$instance_file" .json)
                    # Check if agent has active identity
                    if find "$PROJECT_ROOT/.beads/panes" -name "*.identity" -type f -exec grep -l "\"$agent_name\"" {} \; 2>/dev/null | head -1 | grep -q .; then
                        echo "$agent_name"
                    fi
                fi
            done | sort -u | tr '\n' ' ' | xargs)
            ;;

        @coordinators)
            # Special group: agents with coordinator role
            agents=$(find "$PROJECT_ROOT/.agent-profiles/instances" -name "*.json" -type f 2>/dev/null | while IFS= read -r instance_file; do
                local role=$(jq -r '.role // empty' "$instance_file" 2>/dev/null)
                if [ "$role" = "coordinator" ]; then
                    local agent_name=$(basename "$instance_file" .json)
                    # Check if agent has active identity
                    if find "$PROJECT_ROOT/.beads/panes" -name "*.identity" -type f -exec grep -l "\"$agent_name\"" {} \; 2>/dev/null | head -1 | grep -q .; then
                        echo "$agent_name"
                    fi
                fi
            done | sort -u | tr '\n' ' ' | xargs)
            ;;

        *)
            print_msg RED "Error: Unknown group '$group'"
            echo ""
            return 1
            ;;
    esac

    echo "$agents"
}

#######################################
# Get pane ID for agent in session
# Arguments:
#   $1 - Agent name
# Returns: Pane ID or empty string
#######################################
get_agent_pane() {
    local agent_name="$1"

    # Search all swarm state files for this agent
    local state_files=$(find "$PIDS_DIR" -name "swarm-*.state" -type f 2>/dev/null)

    while IFS= read -r state_file; do
        if [ -f "$state_file" ]; then
            # Check if agent is in this session
            local pane_id=$(jq -r --arg name "$agent_name" '.agents[] | select(.name == $name) | .pane_id' "$state_file" 2>/dev/null || echo "")

            if [ -n "$pane_id" ]; then
                # Verify pane still exists
                if tmux list-panes -a -F "#{pane_id}" 2>/dev/null | grep -q "^$pane_id$"; then
                    echo "$pane_id"
                    return 0
                fi
            fi
        fi
    done <<< "$state_files"

    echo ""
}

#######################################
# Broadcast via tmux to agent
# Arguments:
#   $1 - Agent name
#   $2 - Message
# Returns: 0 if success, 1 if failed
#######################################
broadcast_tmux() {
    local agent_name="$1"
    local message="$2"

    # Get agent's pane ID
    local pane_id=$(get_agent_pane "$agent_name")

    if [ -z "$pane_id" ]; then
        return 1
    fi

    # Format message with broadcast prefix
    local formatted_msg="📢 BROADCAST: $message"

    # Send to tmux pane (as a comment so it doesn't execute)
    tmux send-keys -t "$pane_id" "# $formatted_msg" C-m 2>/dev/null

    return $?
}

#######################################
# Broadcast via mail to agent
# Arguments:
#   $1 - Agent name
#   $2 - Message
#   $3 - Channel (for subject)
# Returns: 0 if success, 1 if failed
#######################################
broadcast_mail() {
    local agent_name="$1"
    local message="$2"
    local channel="${3:-broadcast}"

    if [ ! -f "$MAIL_HELPER" ]; then
        return 1
    fi

    # Use the broadcast command from agent-mail-helper.sh
    "$MAIL_HELPER" broadcast "$agent_name" "$channel" "$message" "normal" >/dev/null 2>&1

    return $?
}

#######################################
# Broadcast to list of agents
# Arguments:
#   $1 - Space-separated agent names
#   $2 - Subject
#   $3 - Message
#   $4 - Group name (for logging)
#   $5 - Delivery mode (both|tmux-only|mail-only)
#   $6 - Importance (normal|urgent)
#   $7 - Dry run flag (true|false)
#######################################
broadcast_to_agents() {
    local agents_str="$1"
    local subject="$2"
    local message="$3"
    local group="$4"
    local delivery_mode="${5:-both}"
    local importance="${6:-normal}"
    local dry_run="${7:-false}"

    if [ -z "$agents_str" ]; then
        print_msg YELLOW "No agents found for group: $group"
        return 1
    fi

    IFS=' ' read -ra agents <<< "$agents_str"

    local total=${#agents[@]}
    local tmux_success=0
    local mail_success=0
    local failed=0

    print_msg BLUE "Broadcasting to $total agent(s) in group: $group"
    echo "  Subject: $subject"
    echo "  Delivery: $delivery_mode"
    echo "  Importance: $importance"

    if [ "$dry_run" = "true" ]; then
        echo -e "\n${YELLOW}DRY RUN - No messages will be sent${NC}\n"
    fi

    echo ""

    for agent_name in "${agents[@]}"; do
        local tmux_ok=false
        local mail_ok=false

        if [ "$dry_run" = "true" ]; then
            # Dry run: just show what would be sent
            echo -e "  ${CYAN}○${NC} $agent_name (would send via $delivery_mode)"
            continue
        fi

        # Send via tmux if enabled
        if [ "$delivery_mode" = "both" ] || [ "$delivery_mode" = "tmux-only" ]; then
            if broadcast_tmux "$agent_name" "$subject: $message"; then
                tmux_ok=true
                ((tmux_success++))
            fi
        fi

        # Send via mail if enabled
        if [ "$delivery_mode" = "both" ] || [ "$delivery_mode" = "mail-only" ]; then
            if broadcast_mail "$agent_name" "$message" "$subject"; then
                mail_ok=true
                ((mail_success++))
            fi
        fi

        # Report delivery status
        if [ "$delivery_mode" = "both" ]; then
            if $tmux_ok && $mail_ok; then
                echo -e "  ${GREEN}✓${NC} $agent_name (tmux + mail)"
            elif $tmux_ok; then
                echo -e "  ${YELLOW}◐${NC} $agent_name (tmux only)"
            elif $mail_ok; then
                echo -e "  ${YELLOW}◐${NC} $agent_name (mail only)"
            else
                echo -e "  ${RED}✗${NC} $agent_name (failed)"
                ((failed++))
            fi
        elif [ "$delivery_mode" = "tmux-only" ]; then
            if $tmux_ok; then
                echo -e "  ${GREEN}✓${NC} $agent_name (tmux)"
            else
                echo -e "  ${RED}✗${NC} $agent_name (failed)"
                ((failed++))
            fi
        elif [ "$delivery_mode" = "mail-only" ]; then
            if $mail_ok; then
                echo -e "  ${GREEN}✓${NC} $agent_name (mail)"
            else
                echo -e "  ${RED}✗${NC} $agent_name (failed)"
                ((failed++))
            fi
        fi
    done

    if [ "$dry_run" = "false" ]; then
        echo ""
        print_msg GREEN "Broadcast complete:"
        echo "  Total agents: $total"

        if [ "$delivery_mode" = "both" ] || [ "$delivery_mode" = "tmux-only" ]; then
            echo "  Tmux delivery: $tmux_success"
        fi

        if [ "$delivery_mode" = "both" ] || [ "$delivery_mode" = "mail-only" ]; then
            echo "  Mail delivery: $mail_success"
        fi

        if [ $failed -gt 0 ]; then
            echo -e "  ${RED}Failed: $failed${NC}"
            return 1
        fi
    fi

    return 0
}

#######################################
# Main function
#######################################
main() {
    # Check for help first
    if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
        usage
        exit 0
    fi

    # Require at least 3 arguments: group, subject, message
    if [ $# -lt 3 ]; then
        print_msg RED "Error: Insufficient arguments"
        echo "Usage: $(basename "$0") <group> <subject> <message> [options]"
        usage
        exit 1
    fi

    # Parse positional arguments
    local group="$1"
    local subject="$2"
    local message="$3"
    shift 3

    # Parse options
    local delivery_mode="both"
    local importance="normal"
    local msg_type=""
    local dry_run="false"

    while [ $# -gt 0 ]; do
        case "$1" in
            --importance)
                if [ $# -lt 2 ]; then
                    print_msg RED "Error: --importance requires a value"
                    exit 1
                fi
                importance="$2"
                shift 2
                ;;
            --type)
                if [ $# -lt 2 ]; then
                    print_msg RED "Error: --type requires a value"
                    exit 1
                fi
                msg_type="$2"
                # Override importance based on message type
                importance=$(get_message_importance "$msg_type")
                shift 2
                ;;
            --mail-only)
                delivery_mode="mail-only"
                shift
                ;;
            --tmux-only)
                delivery_mode="tmux-only"
                shift
                ;;
            --dry-run)
                dry_run="true"
                shift
                ;;
            *)
                print_msg RED "Error: Unknown option '$1'"
                usage
                exit 1
                ;;
        esac
    done

    # Validate group format
    case "$group" in
        @all|@active|@coordinators)
            # Valid group
            ;;
        @swarm:*|@type:*)
            # Valid group with parameter
            ;;
        *)
            print_msg RED "Error: Invalid group '$group'"
            echo "Valid groups: @all, @active, @swarm:<name>, @type:<type>, @coordinators"
            exit 1
            ;;
    esac

    # Resolve group to agent list
    local agents=$(resolve_group "$group")

    if [ -z "$agents" ]; then
        print_msg YELLOW "No agents found for group: $group"
        exit 0
    fi

    # Add message type prefix to subject if specified
    if [ -n "$msg_type" ]; then
        subject="[$msg_type] $subject"
    fi

    # Broadcast to agents
    broadcast_to_agents "$agents" "$subject" "$message" "$group" "$delivery_mode" "$importance" "$dry_run"
}

main "$@"
