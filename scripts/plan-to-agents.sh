#!/usr/bin/env bash
# plan-to-agents.sh - Smart agent spawning from bead queue
#
# Analyzes the bead queue and recommends (or auto-spawns) the right
# number and types of agents.
#
# Usage:
#   ./scripts/plan-to-agents.sh                # Show recommendation
#   ./scripts/plan-to-agents.sh --auto         # Auto-spawn recommended agents
#   ./scripts/plan-to-agents.sh --json         # Output as JSON
#   ./scripts/plan-to-agents.sh --max-agents 6 # Cap total agents
#
# Part of: Autonomous Agent Lifecycle System (bd-3u96)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TYPES_FILE="$PROJECT_ROOT/.agent-profiles/types.yaml"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'
BOLD='\033[1m'

# Defaults
AUTO_SPAWN=false
JSON_OUTPUT=false
MAX_AGENTS=8
SESSION_NAME=""

#######################################
# Parse arguments
#######################################
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --auto)
                AUTO_SPAWN=true
                shift
                ;;
            --json)
                JSON_OUTPUT=true
                shift
                ;;
            --max-agents)
                MAX_AGENTS="$2"
                shift 2
                ;;
            --session)
                SESSION_NAME="$2"
                shift 2
                ;;
            --project)
                PROJECT_ROOT="$2"
                shift 2
                ;;
            --help|-h)
                cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Analyze bead queue and recommend/spawn typed agents.

Options:
  --auto             Auto-spawn recommended agents into tmux
  --json             Output recommendation as JSON
  --max-agents N     Maximum total agents (default: 8)
  --session NAME     Tmux session name for spawning (default: auto)
  --project PATH     Target project directory (default: this project)
  -h, --help         Show this help

How it works:
  1. Queries BV for all ready/prioritized beads
  2. Infers agent type from bead title, description, and labels
  3. Aggregates type counts
  4. Compares against currently active agents
  5. Outputs recommendation (or spawns if --auto)

EOF
                exit 0
                ;;
            *)
                echo -e "${RED}Unknown option: $1${NC}" >&2
                exit 1
                ;;
        esac
    done
}

# Shared type inference function
source "$SCRIPT_DIR/lib-infer-type.sh"

#######################################
# Get ready beads from BV
# Returns: JSON array of beads
#######################################
get_ready_beads() {
    # Run br in the target project directory so it finds the right .beads/*.db
    (
        cd "$PROJECT_ROOT" 2>/dev/null || true
        # Sync first (suppress all output — this runs inside $() capture)
        br sync --flush-only --force >/dev/null 2>&1 || true
        # Get ready beads (unblocked, not deferred)
        br ready --json 2>/dev/null || echo "[]"
    )
}

#######################################
# Get active agents
# Returns: count per type
#######################################
get_active_agents() {
    if [ -f "$SCRIPT_DIR/agent-registry.sh" ]; then
        "$SCRIPT_DIR/agent-registry.sh" active 2>/dev/null || echo "No active agents"
    else
        echo "No active agents"
    fi
}

#######################################
# Main analysis
#######################################
main() {
    parse_args "$@"

    # Get ready beads
    local beads_json
    beads_json=$(get_ready_beads)

    local bead_count
    bead_count=$(echo "$beads_json" | jq 'length' 2>/dev/null || echo "0")

    if [ "$bead_count" = "0" ]; then
        if [ "$JSON_OUTPUT" = true ]; then
            echo '{"beads":0,"recommendation":[],"message":"No ready beads in queue"}'
        else
            echo -e "${YELLOW}No ready beads in the queue.${NC}"
        fi
        exit 0
    fi

    # Analyze each bead and count types
    # Use simple counters (bash 3.2 compatible)
    local count_general=0
    local count_backend=0
    local count_frontend=0
    local count_devops=0
    local count_docs=0
    local count_qa=0

    while IFS= read -r bead; do
        local title
        title=$(echo "$bead" | jq -r '.title // ""')
        local description
        description=$(echo "$bead" | jq -r '.description // ""')
        local labels
        labels=$(echo "$bead" | jq -r '.labels // ""')

        local agent_type
        agent_type=$(infer_agent_type "$title" "$description" "$labels")

        case "$agent_type" in
            general)  count_general=$((count_general + 1)) ;;
            backend)  count_backend=$((count_backend + 1)) ;;
            frontend) count_frontend=$((count_frontend + 1)) ;;
            devops)   count_devops=$((count_devops + 1)) ;;
            docs)     count_docs=$((count_docs + 1)) ;;
            qa)       count_qa=$((count_qa + 1)) ;;
        esac
    done < <(echo "$beads_json" | jq -c '.[]')

    # Calculate recommended agent counts
    # Rule: 1 agent per 2 beads of that type, minimum 1 if any exist, cap at MAX_AGENTS
    recommend_count() {
        local beads=$1
        if [ "$beads" -eq 0 ]; then
            echo 0
        elif [ "$beads" -le 2 ]; then
            echo 1
        else
            echo $(( (beads + 1) / 2 ))
        fi
    }

    local rec_general=$(recommend_count $count_general)
    local rec_backend=$(recommend_count $count_backend)
    local rec_frontend=$(recommend_count $count_frontend)
    local rec_devops=$(recommend_count $count_devops)
    local rec_docs=$(recommend_count $count_docs)
    local rec_qa=$(recommend_count $count_qa)

    local total=$((rec_general + rec_backend + rec_frontend + rec_devops + rec_docs + rec_qa))

    # Cap at MAX_AGENTS (proportionally reduce)
    if [ "$total" -gt "$MAX_AGENTS" ]; then
        local scale
        scale=$(echo "scale=2; $MAX_AGENTS / $total" | bc -l)
        rec_general=$(echo "$rec_general * $scale / 1" | bc)
        rec_backend=$(echo "$rec_backend * $scale / 1" | bc)
        rec_frontend=$(echo "$rec_frontend * $scale / 1" | bc)
        rec_devops=$(echo "$rec_devops * $scale / 1" | bc)
        rec_docs=$(echo "$rec_docs * $scale / 1" | bc)
        rec_qa=$(echo "$rec_qa * $scale / 1" | bc)
        # Ensure at least 1 for non-zero categories
        [ "$count_general" -gt 0 ] && [ "$rec_general" -eq 0 ] && rec_general=1
        [ "$count_backend" -gt 0 ] && [ "$rec_backend" -eq 0 ] && rec_backend=1
        [ "$count_frontend" -gt 0 ] && [ "$rec_frontend" -eq 0 ] && rec_frontend=1
        [ "$count_devops" -gt 0 ] && [ "$rec_devops" -eq 0 ] && rec_devops=1
        [ "$count_docs" -gt 0 ] && [ "$rec_docs" -eq 0 ] && rec_docs=1
        [ "$count_qa" -gt 0 ] && [ "$rec_qa" -eq 0 ] && rec_qa=1
        total=$((rec_general + rec_backend + rec_frontend + rec_devops + rec_docs + rec_qa))
    fi

    # JSON output
    if [ "$JSON_OUTPUT" = true ]; then
        cat <<EOF
{
  "beads": $bead_count,
  "by_type": {
    "general": {"beads": $count_general, "recommended_agents": $rec_general},
    "backend": {"beads": $count_backend, "recommended_agents": $rec_backend},
    "frontend": {"beads": $count_frontend, "recommended_agents": $rec_frontend},
    "devops": {"beads": $count_devops, "recommended_agents": $rec_devops},
    "docs": {"beads": $count_docs, "recommended_agents": $rec_docs},
    "qa": {"beads": $count_qa, "recommended_agents": $rec_qa}
  },
  "total_agents": $total
}
EOF
        if [ "$AUTO_SPAWN" = true ]; then
            do_spawn "$rec_general" "$rec_backend" "$rec_frontend" "$rec_devops" "$rec_docs" "$rec_qa"
        fi
        exit 0
    fi

    # Human-readable output
    echo ""
    echo -e "${BOLD}${CYAN}  Bead Queue Analysis${NC}"
    echo -e "  ════════════════════════════════════════"
    echo -e "  ${BLUE}Ready beads:${NC} $bead_count"
    echo ""
    echo -e "  ${BOLD}Type         Beads  Agents Needed${NC}"
    echo -e "  ──────────  ─────  ─────────────"
    [ "$count_general" -gt 0 ]  && printf "  %-10s  %5d  %d\n" "general" "$count_general" "$rec_general"
    [ "$count_backend" -gt 0 ]  && printf "  %-10s  %5d  %d\n" "backend" "$count_backend" "$rec_backend"
    [ "$count_frontend" -gt 0 ] && printf "  %-10s  %5d  %d\n" "frontend" "$count_frontend" "$rec_frontend"
    [ "$count_devops" -gt 0 ]   && printf "  %-10s  %5d  %d\n" "devops" "$count_devops" "$rec_devops"
    [ "$count_docs" -gt 0 ]     && printf "  %-10s  %5d  %d\n" "docs" "$count_docs" "$rec_docs"
    [ "$count_qa" -gt 0 ]       && printf "  %-10s  %5d  %d\n" "qa" "$count_qa" "$rec_qa"
    echo -e "  ──────────  ─────  ─────────────"
    printf "  %-10s  %5d  %d\n" "TOTAL" "$bead_count" "$total"
    echo ""

    # Show active agents for comparison
    echo -e "  ${BOLD}Currently Active Agents:${NC}"
    local active_output
    active_output=$(get_active_agents)
    if [ "$active_output" = "No active agents" ] || [ -z "$active_output" ]; then
        echo -e "  \033[0;90m(none)\033[0m"
    else
        while read -r line; do
            echo -e "  \033[0;90m$line\033[0m"
        done <<< "$active_output"
    fi
    echo ""

    if [ "$AUTO_SPAWN" = true ]; then
        echo -e "  ${GREEN}Auto-spawning $total agents...${NC}"
        do_spawn "$rec_general" "$rec_backend" "$rec_frontend" "$rec_devops" "$rec_docs" "$rec_qa"
    else
        echo -e "  ${YELLOW}To spawn these agents:${NC}"
        echo -e "  ${CYAN}./scripts/plan-to-agents.sh --auto${NC}"
        echo ""
    fi
}

#######################################
# Spawn agents based on recommendations
#######################################
do_spawn() {
    local rec_general=$1
    local rec_backend=$2
    local rec_frontend=$3
    local rec_devops=$4
    local rec_docs=$5
    local rec_qa=$6

    local session="${SESSION_NAME:-agent-auto-$(date +%s)}"

    # Check if session exists
    if tmux has-session -t "$session" 2>/dev/null; then
        echo -e "${YELLOW}Session '$session' already exists. Use --session to specify a different name.${NC}" >&2
        return 1
    fi

    local total=$((rec_general + rec_backend + rec_frontend + rec_devops + rec_docs + rec_qa))

    if [ "$total" -eq 0 ]; then
        echo -e "${YELLOW}No agents to spawn.${NC}" >&2
        return 0
    fi

    # Create tmux session
    tmux new-session -d -s "$session" -c "$PROJECT_ROOT" -n "agents"

    # Respect tmux pane-base-index setting
    local pane_base
    pane_base=$(tmux show-option -gv pane-base-index 2>/dev/null || echo "0")
    local pane_index=$pane_base
    local first_pane=true

    # Helper to spawn N agents of a type
    spawn_type() {
        local type_name=$1
        local count=$2

        for ((i=0; i<count; i++)); do
            if [ "$first_pane" = true ]; then
                first_pane=false
            else
                tmux split-window -t "$session" -c "$PROJECT_ROOT"
                tmux select-layout -t "$session" tiled
            fi

            # Start agent-runner in this pane
            tmux send-keys -t "$session:agents.$pane_index" \
                "cd '$PROJECT_ROOT' && ./scripts/agent-runner.sh" C-m

            pane_index=$((pane_index + 1))
        done
    }

    [ "$rec_general" -gt 0 ]  && spawn_type "general" "$rec_general"
    [ "$rec_backend" -gt 0 ]  && spawn_type "backend" "$rec_backend"
    [ "$rec_frontend" -gt 0 ] && spawn_type "frontend" "$rec_frontend"
    [ "$rec_devops" -gt 0 ]   && spawn_type "devops" "$rec_devops"
    [ "$rec_docs" -gt 0 ]     && spawn_type "docs" "$rec_docs"
    [ "$rec_qa" -gt 0 ]       && spawn_type "qa" "$rec_qa"

    echo -e "${GREEN}Spawned $total agents in session '$session'${NC}" >&2

    # Open in iTerm2 if available (check LC_TERMINAL for tmux compatibility)
    if [ "${LC_TERMINAL:-}" = "iTerm2" ] || [ "${TERM_PROGRAM:-}" = "iTerm.app" ]; then
        echo -e "${BLUE}Opening session in new iTerm tab...${NC}" >&2
        osascript <<EOF
tell application "iTerm"
    tell current window
        create tab with default profile command "tmux attach -t $session"
    end tell
end tell
EOF
        echo -e "${GREEN}✓ Session opened in new tab${NC}" >&2
    else
        echo -e "${CYAN}Attach: tmux attach -t $session${NC}" >&2
    fi
}

main "$@"
