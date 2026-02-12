#!/bin/bash
# br-start-work: Start work with beads/BV workflow automation
#
# Usage: br-start-work [TITLE] [OPTIONS]
#        br-start-work --claim-existing   # Just claim existing recommended task
#
# Options:
#   --type TYPE          Issue type: task, bug, feature, epic, docs (default: task)
#   --priority PRIORITY  P0-P4 (default: P2)
#   --claim-existing     Skip creation, just claim existing recommendation
#   --no-reserve         Don't reserve files automatically
#   --dry-run            Show what would be done without executing
#
# Examples:
#   br-start-work "Fix login bug" --type bug --priority P1
#   br-start-work                     # Interactive mode or claim existing
#   br-start-work --claim-existing    # Just claim top BV recommendation

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LOG_SCRIPT="$SCRIPT_DIR/log-bead-activity.sh"

# Default values
TITLE=""
TYPE="task"
PRIORITY="P2"
CLAIM_EXISTING=false
RESERVE_FILES=true
DRY_RUN=false
FORCE=false
RESUME_BEAD=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --type)
            TYPE="$2"
            shift 2
            ;;
        --priority)
            PRIORITY="$2"
            shift 2
            ;;
        --claim-existing)
            CLAIM_EXISTING=true
            shift
            ;;
        --no-reserve)
            RESERVE_FILES=false
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --force)
            FORCE=true
            shift
            ;;
        --resume)
            RESUME_BEAD="$2"
            shift 2
            ;;
        --help|-h)
            cat <<EOF
br-start-work: Start work with beads/BV workflow automation

Usage: br-start-work [TITLE] [OPTIONS]
       br-start-work --resume <bead_id>
       br-start-work --claim-existing
       br-start-work --force "Title"

Options:
  --type TYPE          Issue type: task, bug, feature, epic, docs (default: task)
  --priority PRIORITY  P0-P4 (default: P2)
  --claim-existing     Skip creation, just claim existing recommendation
  --no-reserve         Don't reserve files automatically
  --dry-run            Show what would be done without executing
  --force              Skip duplicate bead check (create new even if you have one)
  --resume BEAD_ID     Resume work on existing bead
  --help, -h           Show this help

Examples:
  br-start-work "Fix login bug" --type bug --priority P1
  br-start-work                     # Interactive mode or claim existing
  br-start-work --claim-existing    # Just claim top BV recommendation
  br-start-work --resume bd-1rlq    # Resume work on existing bead
  br-start-work --force "Urgent"    # Create new bead even if you have one active

Workflow:
  1. Checks BV for existing recommendations
  2. If --claim-existing or suitable recommendation exists, claims it
  3. Otherwise creates new bead with given title/type/priority
  4. Updates status to in_progress
  5. Sets BEAD_ID environment variable for mail/commit use
  6. Optionally reserves files (if reserve-files.sh available)

EOF
            exit 0
            ;;
        -*)
            echo "Error: Unknown option $1"
            exit 1
            ;;
        *)
            if [ -z "$TITLE" ]; then
                TITLE="$1"
            else
                echo "Error: Multiple titles provided: '$TITLE' and '$1'"
                exit 1
            fi
            shift
            ;;
    esac
done

# Ensure required tools are installed
for cmd in br bv jq; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: $cmd is required but not installed."
        exit 1
    fi
done

# Get agent name
AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
OS_USER=$(whoami)
if [ "$AGENT_NAME" = "unknown" ] || [ "$AGENT_NAME" = "$OS_USER" ]; then
    echo "❌ Cannot create beads without a registered agent identity." >&2
    echo "   Your agent name resolved to '$AGENT_NAME' which is not valid." >&2
    echo "   Register first: ./scripts/agent-mail-helper.sh register \"Your role\"" >&2
    exit 1
fi

# Check if agent already has an in_progress bead
if [ "$DRY_RUN" = false ] && [ "$FORCE" = false ]; then
    EXISTING_BEAD=$(br list --status in_progress --json 2>/dev/null | jq -r ".[] | select(.owner == \"$AGENT_NAME\") | .id" | head -1)
    if [ -n "$EXISTING_BEAD" ]; then
        echo "⚠️  You already have an active bead: $EXISTING_BEAD"
        br show "$EXISTING_BEAD" 2>/dev/null | head -5
        echo ""
        echo "Options:"
        echo "  1. Continue with existing: export BEAD_ID=$EXISTING_BEAD"
        echo "  2. Close it first: br close $EXISTING_BEAD"
        echo "  3. Force new: br-start-work.sh --force 'Title'"
        exit 0
    fi
fi

# Sync beads data for BV
echo "🔄 Syncing beads data..."
if ! br sync --flush-only --force 2>/dev/null; then
    echo "Warning: Could not sync beads data"
fi

# Handle --resume flag
if [ -n "$RESUME_BEAD" ]; then
    # Validate bead exists
    BEAD_INFO=$(br show "$RESUME_BEAD" --json 2>/dev/null || echo "{}")
    BEAD_STATUS=$(echo "$BEAD_INFO" | jq -r '.[0].status // empty')

    if [ -z "$BEAD_STATUS" ]; then
        echo "❌ Bead $RESUME_BEAD not found"
        exit 1
    fi

    BEAD_ID="$RESUME_BEAD"

    if [ "$DRY_RUN" = true ]; then
        echo "✅ [DRY RUN] Would resume bead: $BEAD_ID"
        echo "   Current status: $BEAD_STATUS"
        # Skip actual operations
    else
        # Update status to in_progress if not already (also set assignee)
        if [ "$BEAD_STATUS" != "in_progress" ]; then
            echo "📝 Updating status to in_progress..."
            if ! br update "$BEAD_ID" --status in_progress --assignee "$AGENT_NAME"; then
                echo "❌ ERROR: Failed to update bead status/assignee" >&2
                exit 1
            fi
        fi

        # Verify assignee before creating tracking file
        verify_assignee=$(br show "$BEAD_ID" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
        if [ "$verify_assignee" != "$AGENT_NAME" ]; then
            echo "❌ ERROR: Claim verification failed for $BEAD_ID" >&2
            echo "   Expected assignee: $AGENT_NAME" >&2
            echo "   Actual assignee: ${verify_assignee:-<none>}" >&2
            echo "   Another agent may have claimed it first." >&2
            exit 1
        fi

        # Set tracking file (only if assignee verified)
        BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"
        echo "$BEAD_ID" > "$BEAD_TRACKING_FILE"
        echo "📝 Bead tracking file: $BEAD_TRACKING_FILE" >&2
    fi

    # Export for use in shell
    export BEAD_ID
    echo ""
    echo "🚀 Resumed work on: $BEAD_ID"
    echo ""
    echo "Next steps:"
    echo "   1. Use BEAD_ID in mail: --thread_id '$BEAD_ID'"
    echo "   2. Use in commits: git commit -m '[$BEAD_ID] Your message'"
    echo "   3. Close when done: br close '$BEAD_ID'"

    exit 0
fi

# Function to claim an existing bead
claim_existing_bead() {
    echo "🤖 Checking BV for existing recommendations..."

    # Get BV recommendation
    BV_OUTPUT=$(bv --robot-next --format json 2>/dev/null || echo "{}")
    TASK_ID=$(echo "$BV_OUTPUT" | jq -r '.id // empty')

    if [ -z "$TASK_ID" ] || [ "$TASK_ID" = "null" ]; then
        echo "📭 No existing recommendations found"
        return 1
    fi

    # Check if already in progress
    IN_PROGRESS=$(echo "$BV_OUTPUT" | jq -r '.reasons[]? // empty' | grep -i "in progress" || true)
    if [ -n "$IN_PROGRESS" ]; then
        echo "⚠️  Task $TASK_ID is already in progress"
        return 1
    fi

    TASK_TITLE=$(echo "$BV_OUTPUT" | jq -r '.title // empty')
    SCORE=$(echo "$BV_OUTPUT" | jq -r '.score // empty')

    echo "🎯 BV recommends: $TASK_ID - $TASK_TITLE"
    echo "   Score: $SCORE"

    if [ "$DRY_RUN" = true ]; then
        echo "✅ [DRY RUN] Would claim: $TASK_ID"
        BEAD_ID="$TASK_ID"
        return 0
    fi

    # Claim the task (set assignee and status)
    echo "📝 Claiming task $TASK_ID..."
    if ! br update "$TASK_ID" --status in_progress --assignee "$AGENT_NAME"; then
        echo "❌ Failed to claim $TASK_ID"
        return 1
    fi

    # Verify the claim succeeded
    verify_assignee=$(br show "$TASK_ID" --json 2>/dev/null | jq -r '.[0].assignee // empty' 2>/dev/null)
    if [ "$verify_assignee" != "$AGENT_NAME" ]; then
        echo "❌ Claim verification failed for $TASK_ID" >&2
        echo "   Expected assignee: $AGENT_NAME" >&2
        echo "   Actual assignee: ${verify_assignee:-<none>}" >&2
        echo "   Another agent may have claimed it first." >&2
        return 1
    fi

    echo "✅ Successfully claimed $TASK_ID (assignee: $AGENT_NAME)"
    BEAD_ID="$TASK_ID"
    # Log claim activity
    if [ -f "$LOG_SCRIPT" ]; then
        "$LOG_SCRIPT" "$BEAD_ID" "claim" "$AGENT_NAME"
    fi
    return 0
}

# Function to create new bead
create_new_bead() {
    local title="$1"
    local type="$2"
    local priority="$3"

    if [ -z "$title" ]; then
        # Interactive prompt for title
        echo "📝 Creating new bead (press Enter for default)"
        echo -n "Title: "
        read -r title
        if [ -z "$title" ]; then
            title="Untitled work $(date '+%Y-%m-%d %H:%M')"
        fi
    fi

    echo "🆕 Creating new bead: $title"
    echo "   Type: $type, Priority: $priority"

    if [ "$DRY_RUN" = true ]; then
        echo "✅ [DRY RUN] Would create bead: $title"
        BEAD_ID="bd-dryrun-$(date +%s)"
        return 0
    fi

    # Create the bead
    CREATED=$(br create --title="$title" --type="$type" --priority="$priority" --assignee="$AGENT_NAME" --json 2>/dev/null || echo "{}")
    NEW_ID=$(echo "$CREATED" | jq -r '.id // empty')

    if [ -z "$NEW_ID" ] || [ "$NEW_ID" = "null" ]; then
        echo "❌ Failed to create bead"
        return 1
    fi

    echo "✅ Created bead: $NEW_ID"

    # Set status to in_progress
    if br update "$NEW_ID" --status in_progress; then
        echo "📝 Set status to in_progress"
        BEAD_ID="$NEW_ID"
        # Log create activity
        if [ -f "$LOG_SCRIPT" ]; then
            "$LOG_SCRIPT" "$BEAD_ID" "create" "$AGENT_NAME"
        fi
        return 0
    else
        echo "⚠️  Created bead but failed to set status"
        BEAD_ID="$NEW_ID"
        # Log create activity (even though status not updated)
        if [ -f "$LOG_SCRIPT" ]; then
            "$LOG_SCRIPT" "$BEAD_ID" "create" "$AGENT_NAME"
        fi
        return 0  # Still created the bead
    fi
}

# Function to reserve files
reserve_related_files() {
    local bead_id="$1"

    if [ "$RESERVE_FILES" = false ]; then
        return 0
    fi

    if [ ! -f "$SCRIPT_DIR/reserve-files.sh" ]; then
        echo "ℹ️  reserve-files.sh not found, skipping file reservation"
        return 0
    fi

    echo "🔒 Reserving files for $bead_id..."

    # Default patterns to reserve (can be expanded)
    PATTERNS=(
        "*.md"
        "scripts/*.sh"
        "panes/*.sh"
    )

    for pattern in "${PATTERNS[@]}"; do
        if [ "$DRY_RUN" = true ]; then
            echo "   [DRY RUN] Would reserve: $pattern"
        else
            "$SCRIPT_DIR/reserve-files.sh" reserve "$pattern" --reason "$bead_id" >/dev/null 2>&1 || true
        fi
    done

    echo "✅ Files reserved (if any matches)"
}

# Main workflow
BEAD_ID=""

if [ "$CLAIM_EXISTING" = true ]; then
    # Try to claim existing, fail if none
    if ! claim_existing_bead; then
        echo "❌ No existing bead to claim (use without --claim-existing to create new)"
        exit 1
    fi
else
    # First try to claim existing
    if claim_existing_bead; then
        echo "✅ Claimed existing recommendation"
    else
        # Create new bead
        if ! create_new_bead "$TITLE" "$TYPE" "$PRIORITY"; then
            echo "❌ Failed to create new bead"
            exit 1
        fi
    fi
fi

# Reserve files if we have a bead ID
if [ -n "$BEAD_ID" ]; then
    reserve_related_files "$BEAD_ID"

    # Write bead tracking file for hooks to check
    AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
    BEAD_TRACKING_FILE="/tmp/agent-bead-${AGENT_NAME}.txt"
    echo "$BEAD_ID" > "$BEAD_TRACKING_FILE"
    echo "📝 Bead tracking file: $BEAD_TRACKING_FILE" >&2

    # Export for use in shell
    export BEAD_ID
    echo ""
    echo "🚀 Ready to work on: $BEAD_ID"
    echo ""
    echo "Next steps:"
    echo "   1. Use BEAD_ID in mail: --thread_id '$BEAD_ID'"
    echo "   2. Use in commits: git commit -m '[$BEAD_ID] Your message'"
    echo "   3. Close when done: br close '$BEAD_ID'"
    echo ""
    echo "BEAD_ID exported to environment. Use 'echo \$BEAD_ID' to see it."
else
    echo "⚠️  No bead ID assigned (dry run mode?)"
fi

# Sync again to update JSONL
if [ "$DRY_RUN" = false ]; then
    br sync --flush-only --force 2>/dev/null || true
fi