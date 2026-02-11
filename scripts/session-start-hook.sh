#!/usr/bin/env bash
# Session Start Hook - Re-register agents after /clear or new session
# Called by Claude Code on session start events

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
MAIL_PROJECT_KEY="${MAIL_PROJECT_KEY:-$PROJECT_ROOT}"

# Read hook input from stdin
INPUT=$(cat)

# Extract the source (startup, resume, clear, compact)
SOURCE=$(echo "$INPUT" | jq -r '.source // "unknown"')

# Log for debugging
echo "[session-start-hook] Session start detected: source=$SOURCE" >&2

# On fresh startup, clear stale identity and re-register
if [ "$SOURCE" = "startup" ]; then
    source "$SCRIPT_DIR/lib/pane-init.sh"
    if init_pane "Claude Agent" >/dev/null 2>&1; then
        echo "[session-start-hook] ✓ Agent registered as: $AGENT_NAME" >&2
    else
        echo "[session-start-hook] ⚠ Warning: Could not register agent" >&2
    fi
else
    # For resume/clear/compact, just verify existing registration
    if "$SCRIPT_DIR/agent-mail-helper.sh" whoami >/dev/null 2>&1; then
        echo "[session-start-hook] ✓ Agent mail registration verified" >&2
    else
        echo "[session-start-hook] ⚠ Warning: Could not verify agent mail registration" >&2
    fi
fi

# Ensure bead-stale-monitor is running (auto-restart if crashed)
if [ -f "$SCRIPT_DIR/bead-stale-monitor.sh" ]; then
    if ! "$SCRIPT_DIR/bead-stale-monitor.sh" status >/dev/null 2>&1; then
        echo "[session-start-hook] ⚠ Bead monitor not running, starting..." >&2
        "$SCRIPT_DIR/bead-stale-monitor.sh" start >/dev/null 2>&1 || true
        echo "[session-start-hook] ✓ Bead monitor started" >&2
    else
        echo "[session-start-hook] ✓ Bead monitor running" >&2
    fi
fi

# Ensure mail monitor is running for this pane (auto-restart if crashed)
if "$SCRIPT_DIR/mail-monitor-ctl.sh" ensure >/dev/null 2>&1; then
    echo "[session-start-hook] ✓ Mail monitor running" >&2
else
    echo "[session-start-hook] ⚠ Could not ensure mail monitor" >&2
fi

# Check for active bead (beads workflow enforcement)
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"

if [ -f "$BEAD_TRACKING_FILE" ]; then
    BEAD_ID=$(cat "$BEAD_TRACKING_FILE")
    echo "[session-start-hook] ✓ Active bead: $BEAD_ID" >&2
else
    # Check if there are recommended beads available
    echo "[session-start-hook] ℹ No active bead claimed" >&2

    # Sync and check BV for recommendations (if bv is available)
    if command -v bv >/dev/null 2>&1; then
        br sync --flush-only 2>/dev/null || true
        RECOMMENDED=$(bv --robot-next 2>/dev/null | jq -r '.id // empty' 2>/dev/null || true)
        if [ -n "$RECOMMENDED" ]; then
            echo "[session-start-hook] 💡 Recommended bead: $RECOMMENDED" >&2
            echo "[session-start-hook] 💡 Claim with: ./scripts/bv-claim.sh" >&2
            echo "[session-start-hook] 💡 Or start new: ./scripts/br-start-work.sh 'Your task title'" >&2
        else
            echo "[session-start-hook] 💡 No beads available. Create one: ./scripts/br-start-work.sh 'Your task title'" >&2
        fi
    fi

    # ADVISORY MODE (Week 1): Just warn, don't block
    # To enable BLOCKING MODE (Week 2+): uncomment the next line
    # exit 2
fi


# ============================================
# AUTO-FIX WRONG BEAD ASSIGNEES
# ============================================
# Validate that current bead assignee matches agent identity
# Auto-fix if mismatch detected (defense-in-depth)

AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "")
CURRENT_BEAD=$(cat /tmp/agent-bead-${AGENT_NAME}.txt 2>/dev/null || echo "")

if [[ -n "$CURRENT_BEAD" ]] && [[ -n "$AGENT_NAME" ]]; then
    ASSIGNEE=$(br show "$CURRENT_BEAD" --json 2>/dev/null | jq -r ".[0].assignee // \"unknown\"" 2>/dev/null || echo "unknown")
    
    if [[ "$ASSIGNEE" != "$AGENT_NAME" ]] && [[ "$ASSIGNEE" != "none" ]] && [[ "$ASSIGNEE" != "unknown" ]]; then
        echo "[session-start-hook] ⚠️  Auto-fixing bead assignment: $CURRENT_BEAD" >&2
        echo "[session-start-hook]    Wrong assignee: $ASSIGNEE → Correct: $AGENT_NAME" >&2
        "$SCRIPT_DIR/br-wrapper.sh" update "$CURRENT_BEAD" --assignee "$AGENT_NAME" 2>/dev/null || echo "[session-start-hook] ⚠️  Failed to auto-fix assignee" >&2
    fi
fi

# Exit 0 to allow session to continue
exit 0
