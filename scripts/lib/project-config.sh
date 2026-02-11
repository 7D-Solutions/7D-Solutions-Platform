#!/bin/bash
# Shared project configuration - source this in all scripts
# This allows agent-flywheel to work with any project directory

# Detect project root (prefer env var, fallback to current directory)
PROJECT_ROOT="${PROJECT_ROOT:-$(pwd)}"

# Validate PROJECT_ROOT exists and is accessible
if [ ! -d "$PROJECT_ROOT" ]; then
    echo "ERROR: PROJECT_ROOT directory does not exist: $PROJECT_ROOT" >&2
    echo "Set PROJECT_ROOT environment variable to a valid directory." >&2
    return 1 2>/dev/null || exit 1
fi

if [ ! -w "$PROJECT_ROOT" ]; then
    echo "ERROR: PROJECT_ROOT directory is not writable: $PROJECT_ROOT" >&2
    echo "Check permissions for this directory." >&2
    return 1 2>/dev/null || exit 1
fi

# Set mail project key to current project (for agent isolation)
MAIL_PROJECT_KEY="${MAIL_PROJECT_KEY:-$PROJECT_ROOT}"

# Derived directories
PIDS_DIR="$PROJECT_ROOT/pids"
PANES_DIR="$PROJECT_ROOT/panes"
WORKFLOWS_DIR="$PROJECT_ROOT/workflows"
STATE_DIR="$PROJECT_ROOT/state"
LOGS_DIR="$PROJECT_ROOT/.ntm/logs"

# Scripts location (always in agent-flywheel repo)
# Detect dynamically if not set
if [ -z "${SCRIPTS_DIR:-}" ]; then
    # Try to detect from current script location
    if [ -n "${BASH_SOURCE[0]}" ]; then
        DETECTED_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
        SCRIPTS_DIR="$DETECTED_SCRIPT_DIR"
    else
        # Fallback: This should rarely happen, but if it does, fail gracefully
        echo "ERROR: Could not detect SCRIPTS_DIR location" >&2
        echo "Please set SCRIPTS_DIR environment variable to the agent-flywheel scripts directory" >&2
        return 1 2>/dev/null || exit 1
    fi
fi

# Validate SCRIPTS_DIR exists
if [ ! -d "$SCRIPTS_DIR" ]; then
    echo "WARNING: SCRIPTS_DIR does not exist: $SCRIPTS_DIR" >&2
    echo "Agent flywheel scripts may not be found." >&2
    echo "Set SCRIPTS_DIR environment variable or check installation." >&2
fi

# Ensure all required directories exist
init_project_directories() {
    if ! mkdir -p "$PROJECT_ROOT"/{pids,panes,workflows,state/{snapshots,logs},.ntm/logs,.agent-workflows,.active-agents,.agent-coordination/status} 2>/dev/null; then
        echo "ERROR: Failed to create project directories in $PROJECT_ROOT" >&2
        echo "Check permissions and ensure the path is valid." >&2
        return 1
    fi
}

# Call initialization on source
init_project_directories

# Export for use in subprocesses
export PROJECT_ROOT MAIL_PROJECT_KEY PIDS_DIR PANES_DIR WORKFLOWS_DIR STATE_DIR SCRIPTS_DIR LOGS_DIR
