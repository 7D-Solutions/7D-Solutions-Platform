#!/usr/bin/env bash
# Pre-Edit Check Hook - Claude Code hook wrapper
# Called by Claude Code before Edit tool execution
# Reads hook JSON from stdin, extracts file path, runs pre-edit checks
# Also enforces beads workflow - requires active bead before editing

# Note: -e flag omitted to handle jq parsing errors explicitly
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PRE_EDIT_CHECK="$SCRIPT_DIR/pre-edit-check.sh"
LOG_SCRIPT="$SCRIPT_DIR/log-bead-activity.sh"

# Read hook input from stdin
INPUT=$(cat)

# Extract file path being edited from tool_input
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# If no file path, allow (shouldn't happen, but defensive)
if [ -z "$FILE_PATH" ]; then
    exit 0
fi

# Allow config toggle files without a bead
BASENAME=$(basename "$FILE_PATH")
case "$BASENAME" in
    .no-clear|.exit-lifecycle)
        exit 0
        ;;
esac

# ============================================
# ============================================
# BLOCK DIRECT BEADS DATABASE EDITS
# ============================================
# Never allow direct editing of .beads/issues.jsonl
# Only the 'br' command should modify this file
if [[ "$FILE_PATH" == *"/.beads/issues.jsonl" ]]; then
    echo "❌ ERROR: Direct editing of .beads/issues.jsonl is forbidden" >&2
    echo "" >&2
    echo "   The beads database must only be modified through the 'br' command." >&2
    echo "" >&2
    echo "   Common operations:" >&2
    echo "   → br update <bead-id> --assignee \$(./scripts/agent-mail-helper.sh whoami)" >&2
    echo "   → br close <bead-id>" >&2
    echo "   → br create \"Title\"" >&2
    echo "" >&2
    echo "   This prevents corruption and ensures proper workflow tracking." >&2
    exit 2
fi

# ============================================
# BEADS WORKFLOW ENFORCEMENT
# ============================================
# Check if agent has an active bead before allowing edits
#
# Two discovery methods (checked in order):
# 1. AGENT_RUNNER_BEAD env var — set by agent-runner.sh, inherited by spawned claude
# 2. /tmp/agent-bead-{AGENT_NAME}.txt tracking file — for standalone sessions

AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")

# ENFORCEMENT_MODE: "advisory" (warn) or "blocking" (block edits)
ENFORCEMENT_MODE="blocking"  # Changed to "blocking" for Week 2+

# Method 1: Check env var from agent-runner (avoids identity mismatch)
BEAD_ID="${AGENT_RUNNER_BEAD:-}"

# Method 2: Check tracking file by agent name
if [ -z "$BEAD_ID" ]; then
    BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"
    if [ -f "$BEAD_TRACKING_FILE" ]; then
        BEAD_ID=$(cat "$BEAD_TRACKING_FILE")
    fi
fi

if [ -z "$BEAD_ID" ]; then
    echo "⚠️  No active bead claimed!" >&2
    echo "   Before editing, claim or create a bead:" >&2
    echo "   → ./scripts/bv-claim.sh              (claim recommended)" >&2
    echo "   → ./scripts/br-start-work.sh 'Title' (create new)" >&2
    echo "" >&2

    if [ "$ENFORCEMENT_MODE" = "blocking" ]; then
        echo "❌ Edit blocked: Claim a bead first" >&2
        # Log edit blocked
        if [ -f "$LOG_SCRIPT" ]; then
            "$LOG_SCRIPT" "none" "edit_blocked" "$AGENT_NAME"
        fi
        exit 2
    else
        echo "ℹ️  Proceeding anyway (advisory mode)" >&2
        # Log edit allowed without bead (advisory mode)
        if [ -f "$LOG_SCRIPT" ]; then
            "$LOG_SCRIPT" "none" "edit_allowed_without_bead" "$AGENT_NAME"
        fi
    fi
else
    echo "✓ Active bead: $BEAD_ID" >&2
    # Log edit allowed with bead
    if [ -f "$LOG_SCRIPT" ]; then
        "$LOG_SCRIPT" "$BEAD_ID" "edit_allowed" "$AGENT_NAME"
    fi
fi

# ============================================
# FILE RESERVATION CHECK (existing logic)
# ============================================
# Run pre-edit check on the file
# Exit codes: 0=available, 1=reserved, 2=error
if "$PRE_EDIT_CHECK" "$FILE_PATH" >&2; then
    # File available, allow edit
    exit 0
else
    EXIT_CODE=$?
    # File reserved or error occurred
    # Exit code 2 blocks the tool in Claude Code
    exit 2
fi
