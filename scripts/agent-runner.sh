#!/usr/bin/env bash
# agent-runner.sh - Autonomous agent lifecycle launcher
#
# Launches claude once with instructions to cycle through beads autonomously.
# Claude stays running — it closes a bead, claims the next, and keeps going.
# Agent-runner only restarts claude on crashes (not between beads).
#
# Usage:
#   ./scripts/agent-runner.sh                    # Auto-detect identity, claim next bead
#   ./scripts/agent-runner.sh --bead bd-xxx      # Start with specific bead
#   ./scripts/agent-runner.sh --max-restarts 3   # Max crash restarts before giving up
#   ./scripts/agent-runner.sh --dry-run          # Show what would happen
#
# Environment:
#
# Part of: Autonomous Agent Lifecycle System (bd-3u96)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
METRICS_FILE="$PROJECT_ROOT/.beads/runner-cycles.jsonl"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m'
BOLD='\033[1m'

# State
RESTART_COUNT=0
SHUTTING_DOWN=false

# Arguments
TARGET_BEAD=""
MAX_RESTARTS=5
DRY_RUN=false

#######################################
# Print banner
#######################################
print_banner() {
    echo -e "${CYAN}"
    echo "  ╔═══════════════════════════════════════════════╗"
    echo "  ║          Agent Runner - Lifecycle Loop        ║"
    echo "  ║   Continuous bead cycling · Clear on close    ║"
    echo "  ╚═══════════════════════════════════════════════╝"
    echo -e "${NC}"
}

#######################################
# Log to stderr with timestamp
#######################################
log() {
    local level="$1"
    shift
    local color="$NC"
    case "$level" in
        INFO)  color="$GREEN" ;;
        WARN)  color="$YELLOW" ;;
        ERROR) color="$RED" ;;
        DEBUG) color="$GRAY" ;;
    esac
    echo -e "${color}[$(date '+%H:%M:%S')] [$level] $*${NC}" >&2
}

#######################################
# Log a launch/exit event to metrics JSONL
#######################################
log_metric() {
    local event="$1"
    local detail="$2"

    mkdir -p "$(dirname "$METRICS_FILE")"

    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    printf '{"timestamp":"%s","agent":"%s","event":"%s","detail":"%s","restart_count":%s}\n' \
        "$timestamp" "$AGENT_NAME" "$event" "$detail" "$RESTART_COUNT" \
        >> "$METRICS_FILE"
}

#######################################
# Clean up on exit
#######################################
cleanup() {
    SHUTTING_DOWN=true
    log INFO "Shutting down agent runner..."

    # Stop the mail monitor (agent identity is gone, monitor no longer needed)
    "$SCRIPT_DIR/mail-monitor-ctl.sh" stop >/dev/null 2>&1 || true

    # Clean up temp files
    rm -f "/tmp/agent-runner-prompt-${AGENT_NAME}.md" 2>/dev/null
    rm -f "/tmp/agent-bead-${AGENT_NAME}.txt" 2>/dev/null

    log INFO "Agent runner stopped."
    exit 0
}

trap cleanup SIGTERM SIGINT SIGHUP

#######################################
# Wait for bead availability (trigger file or timeout)
#######################################
# Parse arguments
#######################################
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --bead)
                TARGET_BEAD="$2"
                shift 2
                ;;
            --max-restarts)
                MAX_RESTARTS="$2"
                shift 2
                ;;
            --dry-run)
                DRY_RUN=true
                shift
                ;;
            --help|-h)
                cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Launches claude to work through beads autonomously. Claude stays running,
cycling through beads (close → claim next → work). Agent-runner only
restarts claude on crashes.

Options:
  --bead BD-ID         Start with specific bead (skip initial claim)
  --max-restarts N     Max crash restarts before giving up (default: 5)
  --dry-run            Show what would happen without running claude
  -h, --help           Show this help

Behavior:
  Always restarts claude on exit. Ctrl+C to stop.
  If a bead is available, claude launches with it.
  If no beads, claude launches bare (check inbox/mail for work).

EOF
                exit 0
                ;;
            *)
                log ERROR "Unknown option: $1"
                exit 1
                ;;
        esac
    done
}

#######################################
# Determine agent identity
#######################################
get_agent_identity() {
    AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "")
    local os_user
    os_user=$(whoami)

    if [ -z "$AGENT_NAME" ] || [ "$AGENT_NAME" = "$os_user" ]; then
        log ERROR "No agent identity found. Register first:"
        log ERROR "  ./scripts/agent-mail-helper.sh register \"Your role\""
        exit 1
    fi

    log INFO "Agent identity: $AGENT_NAME"
}

#######################################
# Claim next bead from BV
# Returns: bead ID via stdout, or empty string
#######################################
claim_next_bead() {
    log INFO "Checking BV for next recommended bead..."

    # Sync beads data
    br sync --flush-only --force >/dev/null 2>&1 || true

    # Get recommendation
    local bv_output
    bv_output=$(bv --robot-next --format json 2>/dev/null || echo "{}")

    local task_id
    task_id=$(echo "$bv_output" | jq -r '.id // empty' 2>/dev/null)

    if [ -z "$task_id" ] || [ "$task_id" = "null" ]; then
        echo ""
        return
    fi

    local task_title
    task_title=$(echo "$bv_output" | jq -r '.title // "untitled"' 2>/dev/null)

    # In dry-run mode, don't actually claim
    if [ "$DRY_RUN" = true ]; then
        log INFO "[DRY RUN] Would claim bead: $task_id - $task_title"
        echo "$task_id"
        return
    fi

    # Try to claim BV recommendation
    local claimed_bead
    claimed_bead=$(try_claim_bead "$task_id" "$task_title")

    if [ -n "$claimed_bead" ]; then
        echo "$claimed_bead"
        return
    fi

    # BV recommendation failed, try beads from br ready
    log INFO "BV recommendation unavailable, trying br ready..."
    local ready_beads
    ready_beads=$(br ready --json 2>/dev/null | jq -r '.[].id' 2>/dev/null || echo "")

    if [ -z "$ready_beads" ]; then
        log INFO "No ready beads available"
        echo ""
        return
    fi

    # Try each ready bead until we successfully claim one
    local bead_id
    while IFS= read -r bead_id; do
        [ -z "$bead_id" ] && continue

        local bead_title
        bead_title=$(br show "$bead_id" --json 2>/dev/null | jq -r '.[0].title // "untitled"' 2>/dev/null)

        claimed_bead=$(try_claim_bead "$bead_id" "$bead_title")
        if [ -n "$claimed_bead" ]; then
            echo "$claimed_bead"
            return
        fi
    done <<< "$ready_beads"

    log INFO "All ready beads are claimed or unavailable"
    echo ""
}

#######################################
# Try to claim a specific bead
# Args: bead_id, bead_title
# Returns: bead_id if successful, empty string otherwise
#######################################
try_claim_bead() {
    local task_id="$1"
    local task_title="${2:-untitled}"

    # Check if already in progress by someone else
    local task_status
    task_status=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].status // empty' 2>/dev/null)
    if [ "$task_status" = "in_progress" ]; then
        local current_owner
        current_owner=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
        if [ -n "$current_owner" ] && [ "$current_owner" != "$AGENT_NAME" ]; then
            log WARN "Bead $task_id already claimed by $current_owner, skipping"
            echo ""
            return
        fi
    fi

    # Claim it
    claim_output=$(br update "$task_id" --status in_progress --owner "$AGENT_NAME" --assignee "$AGENT_NAME" 2>&1)
    claim_status=$?

    if [ $claim_status -eq 0 ]; then
        local verify_owner
        verify_owner=$(br show "$task_id" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
        if [ "$verify_owner" = "$AGENT_NAME" ]; then
            log INFO "Claimed bead: $task_id - $task_title"
            echo "$task_id"
            return
        else
            log WARN "Lost race for $task_id (owned by $verify_owner)"
            echo ""
            return
        fi
    fi

    log WARN "Failed to claim $task_id: $claim_output"
    echo ""
}

#######################################
# Get bead details for initial message
# Arguments: $1 = bead ID
#######################################
get_bead_details() {
    local bead_id="$1"
    local bead_json
    bead_json=$(br show "$bead_id" --json 2>/dev/null || echo "[]")

    local title description priority labels parent
    local how_to_think acceptance_criteria files_create files_modify verification

    title=$(echo "$bead_json" | jq -r '.[0].title // "Unknown task"' 2>/dev/null)
    description=$(echo "$bead_json" | jq -r '.[0].description // ""' 2>/dev/null)
    priority=$(echo "$bead_json" | jq -r '.[0].priority // ""' 2>/dev/null)
    labels=$(echo "$bead_json" | jq -r '.[0].labels // ""' 2>/dev/null)
    parent=$(echo "$bead_json" | jq -r '.[0].parent // empty' 2>/dev/null)
    how_to_think=$(echo "$bead_json" | jq -r '.[0].how_to_think // empty' 2>/dev/null)
    acceptance_criteria=$(echo "$bead_json" | jq -r '.[0].acceptance_criteria // empty | if type == "array" then join("\n  - ") else . end' 2>/dev/null)
    files_create=$(echo "$bead_json" | jq -r '.[0].files_to_create // empty | if type == "array" then join(", ") else . end' 2>/dev/null)
    files_modify=$(echo "$bead_json" | jq -r '.[0].files_to_modify // empty | if type == "array" then join(", ") else . end' 2>/dev/null)
    verification=$(echo "$bead_json" | jq -r '.[0].verification // empty | if type == "array" then join("\n  - ") else . end' 2>/dev/null)

    echo "Work on bead $bead_id."
    echo ""
    echo "**$title**"
    [ -n "$description" ] && echo "Description: $description"
    [ -n "$priority" ] && echo "Priority: $priority"
    [ -n "$labels" ] && echo "Labels: $labels"
    [ -n "$parent" ] && echo "Parent bead: $parent"
    if [ -n "$how_to_think" ]; then
        echo ""
        echo "## Mindset"
        echo "$how_to_think"
    fi
    if [ -n "$acceptance_criteria" ]; then
        echo ""
        echo "## Acceptance Criteria"
        echo "  - $acceptance_criteria"
    fi
    [ -n "$files_create" ] && echo "Files to create: $files_create"
    [ -n "$files_modify" ] && echo "Files to modify: $files_modify"
    if [ -n "$verification" ]; then
        echo ""
        echo "## Verification"
        echo "  - $verification"
    fi
}

#######################################
# Build system prompt with cycling instructions
# This persists across /clear — it's the durable agent behavior
#######################################
build_system_prompt() {
    local bead_id="$1"
    cat <<PROMPT
You are $AGENT_NAME, an autonomous agent working through beads (tasks).

## Executing a Bead

1. Read the bead details. The Mindset section defines your approach — follow it.
2. Check mail: \`\$PROJECT_ROOT/scripts/agent-mail-helper.sh inbox\`
3. Read acceptance criteria — these are your definition of done.
4. Stay in scope: only touch files listed in files_to_create and files_to_modify. If you need to change other files, that's a new bead.
5. Implement. Commit often with \`[BEAD-ID]\` prefix. Small, focused commits.
6. Run the verification commands. All tests must be integrated — real databases, real APIs. No mocking. No stubs. If a verification step uses mocks, rewrite it against real services.
7. When all acceptance criteria pass and verification succeeds: \`br close BEAD-ID --suggest-next\`
8. Run \`\$PROJECT_ROOT/scripts/next-bead.sh\` and stop. Context will be cleared automatically.

## Scope Rules

- One bead = one concern. Do not mix refactors with features.
- If you discover a separate issue: create a child bead with \`\$PROJECT_ROOT/scripts/br-create.sh "Fix: description" --parent BEAD-ID\`, fix it, close it.
- A bead is only done when all its child beads are also closed.
- Run \`br sync\` before closing to commit bead metadata.

## When to Escalate

- You are blocked and cannot resolve it from the bead text — mail the orchestrator.
- The bead description is ambiguous or contradicts the codebase — mail the orchestrator. Do not guess.
- You have sent two messages on the same topic without resolution — stop and wait.

## Rules

- Work autonomously. Do not ask the user what to do.
- Do not spawn subagents. Do all work directly.
- Always use \`\$PROJECT_ROOT/scripts/...\` for scripts, never \`./scripts/...\`.
- After closing a bead, always run \`\$PROJECT_ROOT/scripts/next-bead.sh\` and stop working.
PROMPT
}

#######################################
# Write bead tracking file (for pre-edit hook fallback)
# Arguments: $1 = bead ID
#######################################
write_tracking_file() {
    local bead_id="$1"
    local tracking_file="/tmp/agent-bead-${AGENT_NAME}.txt"
    echo "$bead_id" > "$tracking_file"
    log DEBUG "Tracking file written: $tracking_file"
}

#######################################
# Main
#######################################
main() {
    parse_args "$@"
    get_agent_identity

    print_banner
    log INFO "Starting agent runner for $AGENT_NAME"
    log INFO "Project: $PROJECT_ROOT"
    log INFO "Max restarts: $MAX_RESTARTS"

    while true; do
        # Find a bead to start with
        local bead_id=""

        if [ -n "$TARGET_BEAD" ]; then
            bead_id="$TARGET_BEAD"
            TARGET_BEAD=""  # Only use once
        else
            bead_id=$(claim_next_bead)
        fi

        if [ -z "$bead_id" ]; then
            log INFO "No beads available. Launching claude without a bead..."
            bead_id=""
        fi

        # Build launch args based on whether we have a bead
        local system_prompt=""
        local initial_message=""

        if [ -n "$bead_id" ]; then
            write_tracking_file "$bead_id"
            system_prompt=$(build_system_prompt "$bead_id")
            initial_message=$(get_bead_details "$bead_id")
        fi

        if [ "$DRY_RUN" = true ]; then
            log INFO "[DRY RUN] Would launch claude with:"
            echo "--- SYSTEM PROMPT ---"
            echo "$system_prompt"
            echo "--- INITIAL MESSAGE ---"
            echo "$initial_message"
            echo "---"
            break
        fi

        # Ensure mail monitor is running before launching claude
        if "$SCRIPT_DIR/mail-monitor-ctl.sh" ensure >/dev/null 2>&1; then
            log INFO "Mail monitor: running"
        else
            log WARN "Mail monitor: could not ensure (notifications may not work)"
        fi

        # Launch claude in FOREGROUND — user can interact in the pane
        log INFO "Launching claude${bead_id:+ (bead: $bead_id)}..."
        log_metric "launch" "${bead_id:-no-bead}"

        local exit_code=0
        local claude_args=(--dangerously-skip-permissions)
        [ -n "$system_prompt" ] && claude_args+=(--append-system-prompt "$system_prompt")

        AGENT_RUNNER_BEAD="${bead_id:-}" \
        PROJECT_ROOT="$PROJECT_ROOT" \
        claude \
            "${claude_args[@]}" \
            ${initial_message:+"$initial_message"} || exit_code=$?

        if [ "$SHUTTING_DOWN" = true ]; then
            break
        fi

        log INFO "claude exited (code: $exit_code)"
        log_metric "exit" "code=$exit_code"

        # Always restart — Ctrl+C (SHUTTING_DOWN) is the only way to stop
        RESTART_COUNT=$((RESTART_COUNT + 1))

        if [ "$RESTART_COUNT" -ge "$MAX_RESTARTS" ]; then
            log ERROR "Reached max restarts ($MAX_RESTARTS). Giving up."
            log_metric "max_restarts" "giving_up"
            break
        fi

        log INFO "Restarting claude (restart $RESTART_COUNT/$MAX_RESTARTS)..."
    done

    log INFO "Agent runner finished."
}

main "$@"
