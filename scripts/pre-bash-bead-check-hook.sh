#!/usr/bin/env bash
# pre-bash-bead-check-hook.sh - PreToolUse hook enforcing bead requirement for Bash commands
#
# Fires on Bash matcher. If the agent has no active bead, checks the command
# against an allowlist of safe/read-only commands. Blocks (or warns in advisory
# mode) if any command in a pipe/chain is not on the allowlist.
#
# Bead discovery (same as pre-edit-check-hook.sh):
#   1. AGENT_RUNNER_BEAD env var (set by agent-runner.sh)
#   2. /tmp/agent-bead-{AGENT_NAME}.txt tracking file
#
# Hook config (in ~/.claude/settings.json):
#   "PreToolUse": [
#     { "matcher": "Bash", "hooks": [{ "type": "command",
#       "command": "\"$CLAUDE_PROJECT_DIR\"/scripts/pre-bash-bead-check-hook.sh",
#       "timeout": 10 }] }
#   ]

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LOG_FILE="${PROJECT_DIR}/.beads/bash-enforcement-log.jsonl"

# Advisory mode: "advisory" = warn + log, "blocking" = block the command
ENFORCEMENT_MODE="advisory"

# ============================================
# Read hook input
# ============================================
INPUT=$(cat)

COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)

if [ -z "$COMMAND" ]; then
    exit 0
fi

# ============================================
# Check for active bead
# ============================================
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")

BEAD_ID="${AGENT_RUNNER_BEAD:-}"

if [ -z "$BEAD_ID" ]; then
    BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"
    if [ -f "$BEAD_TRACKING_FILE" ]; then
        BEAD_ID=$(cat "$BEAD_TRACKING_FILE")
    fi
fi

# If agent has a bead, allow everything
if [ -n "$BEAD_ID" ]; then
    exit 0
fi

# ============================================
# No bead — check command against allowlist
# ============================================

# Allowlisted standalone commands (basename matching)
ALLOWED_COMMANDS=(
    # Bead management
    bv br bv-claim.sh br-start-work.sh br-create.sh next-bead.sh br-create br-start-work bv-claim
    br-wrapper.sh bv-all-open.sh bv-all.sh bv-open.sh bv-sync.sh

    # Identity / mail
    agent-mail-helper.sh whoami

    # Read-only shell
    ls pwd echo date cat head tail grep find wc which type env printenv uname hostname

    # Text processing
    jq sort uniq tr cut sed awk

    # Shell builtins
    true false test "[" "[[" printf

    # Misc safe
    basename dirname realpath
)

# Git subcommands that are safe (read-only)
ALLOWED_GIT_SUBCMDS=(
    status log diff branch remote config show tag describe rev-parse stash ls-files
    name-rev shortlog reflog rev-list for-each-ref
)

# Tmux subcommands that are safe
ALLOWED_TMUX_SUBCMDS=(
    list-panes display-message send-keys list-sessions list-windows has-session
)

# Curl write flags that make it non-read-only
CURL_WRITE_FLAGS=(-X --request -d --data --data-raw --data-binary --data-urlencode -F --form -T --upload-file --json)

# ============================================
# Command parsing helpers
# ============================================

# Extract the base command name from a command string
# Handles paths like ./scripts/foo.sh → foo.sh, /usr/bin/env bash → bash
get_base_cmd() {
    local segment="$1"
    # Trim leading whitespace
    segment="${segment#"${segment%%[![:space:]]*}"}"
    # Skip env var assignments (FOO=bar cmd)
    while [[ "$segment" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; do
        segment="${segment#*=}"
        # Skip the value (handle quoted values)
        segment="${segment#\"*\"}"
        segment="${segment#\'*\'}"
        segment="${segment#[^ ]*}"
        segment="${segment#"${segment%%[![:space:]]*}"}"
    done
    # Get first word
    local first_word
    first_word=$(echo "$segment" | awk '{print $1}')
    # Strip path to get basename
    basename "$first_word" 2>/dev/null || echo "$first_word"
}

# Get the second word (subcommand) from a command string
get_subcmd() {
    local segment="$1"
    segment="${segment#"${segment%%[![:space:]]*}"}"
    # Skip env var assignments
    while [[ "$segment" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; do
        segment="${segment#*=}"
        segment="${segment#\"*\"}"
        segment="${segment#\'*\'}"
        segment="${segment#[^ ]*}"
        segment="${segment#"${segment%%[![:space:]]*}"}"
    done
    echo "$segment" | awk '{print $2}'
}

# Check if a value is in an array
in_array() {
    local needle="$1"
    shift
    for item in "$@"; do
        if [ "$needle" = "$item" ]; then
            return 0
        fi
    done
    return 1
}

# Check if curl command is read-only (no write flags)
is_curl_readonly() {
    local segment="$1"
    for flag in "${CURL_WRITE_FLAGS[@]}"; do
        if echo "$segment" | grep -qE -- "(^|\\s)${flag}(\\s|=|\$)"; then
            return 1
        fi
    done
    return 0
}

# Check a single command segment against the allowlist
# Returns 0 if allowed, 1 if not
check_segment() {
    local segment="$1"
    # Trim whitespace
    segment="${segment#"${segment%%[![:space:]]*}"}"

    # Empty segment is fine
    if [ -z "$segment" ]; then
        return 0
    fi

    local base_cmd
    base_cmd=$(get_base_cmd "$segment")

    # Handle special cases
    case "$base_cmd" in
        git)
            local subcmd
            subcmd=$(get_subcmd "$segment")
            if in_array "$subcmd" "${ALLOWED_GIT_SUBCMDS[@]}"; then
                return 0
            fi
            return 1
            ;;
        tmux)
            local subcmd
            subcmd=$(get_subcmd "$segment")
            if in_array "$subcmd" "${ALLOWED_TMUX_SUBCMDS[@]}"; then
                return 0
            fi
            return 1
            ;;
        curl)
            if is_curl_readonly "$segment"; then
                return 0
            fi
            return 1
            ;;
        *)
            if in_array "$base_cmd" "${ALLOWED_COMMANDS[@]}"; then
                return 0
            fi
            return 1
            ;;
    esac
}

# ============================================
# Split command into segments and check each
# ============================================
# Replace pipe, &&, ||, ; with newlines, then check each segment
# Note: This is a simplified parser that won't handle all edge cases
# (e.g. pipes inside quotes), but covers the common patterns.

# Use awk to split on |, &&, ||, ; while being careful about quotes
# Simple approach: replace delimiters with a unique separator
SEPARATOR=$'\x01'
SEGMENTS=$(echo "$COMMAND" | sed \
    -e "s/&&/${SEPARATOR}/g" \
    -e "s/||/${SEPARATOR}/g" \
    -e "s/|/${SEPARATOR}/g" \
    -e "s/;/${SEPARATOR}/g")

BLOCKED=false
BLOCKED_CMD=""

while IFS= read -r segment; do
    if ! check_segment "$segment"; then
        BLOCKED=true
        BLOCKED_CMD="$segment"
        break
    fi
done <<< "$(echo "$SEGMENTS" | tr "$SEPARATOR" '\n')"

# ============================================
# If all commands allowed, exit cleanly
# ============================================
if [ "$BLOCKED" = false ]; then
    exit 0
fi

# ============================================
# Command not on allowlist — enforce
# ============================================
BLOCKED_CMD_TRIMMED="${BLOCKED_CMD#"${BLOCKED_CMD%%[![:space:]]*}"}"

# Log to enforcement log
mkdir -p "$(dirname "$LOG_FILE")"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
LOG_ENTRY=$(jq -c -n \
    --arg ts "$TIMESTAMP" \
    --arg agent "$AGENT_NAME" \
    --arg mode "$ENFORCEMENT_MODE" \
    --arg command "$COMMAND" \
    --arg blocked_segment "$BLOCKED_CMD_TRIMMED" \
    '{timestamp: $ts, agent: $agent, mode: $mode, command: $command, blocked_segment: $blocked_segment}')
echo "$LOG_ENTRY" >> "$LOG_FILE"

if [ "$ENFORCEMENT_MODE" = "blocking" ]; then
    echo "⚠️  No active bead — Bash command blocked!" >&2
    echo "   Blocked segment: $BLOCKED_CMD_TRIMMED" >&2
    echo "   Claim a bead first:" >&2
    echo "   → ./scripts/bv-claim.sh              (claim recommended)" >&2
    echo "   → ./scripts/br-start-work.sh 'Title' (create new)" >&2
    exit 2
else
    echo "⚠️  No active bead — Bash command allowed (advisory mode)" >&2
    echo "   Would block: $BLOCKED_CMD_TRIMMED" >&2
    echo "   Claim a bead: ./scripts/bv-claim.sh or ./scripts/br-start-work.sh 'Title'" >&2
    exit 0
fi
