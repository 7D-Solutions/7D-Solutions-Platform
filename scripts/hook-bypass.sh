#!/bin/bash
# Claude Code Hook Bypass Utility
# Use this to temporarily bypass Claude Code hooks for testing

# Get the project root (parent of scripts directory)
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BYPASS_FILE="${PROJECT_ROOT}/.claude-hooks-bypass"

function enable_bypass() {
    touch "$BYPASS_FILE"
    echo "✓ Hook bypass ENABLED"
    echo "  Bypass file: $BYPASS_FILE"
    echo ""
    echo "Hooks will check for this file and exit early if it exists."
}

function disable_bypass() {
    rm -f "$BYPASS_FILE"
    echo "✓ Hook bypass DISABLED"
    echo "  Hooks will run normally."
}

function status() {
    if [ -f "$BYPASS_FILE" ]; then
        echo "⚠️  Hook bypass is ACTIVE"
        echo "   File: $BYPASS_FILE"
    else
        echo "✓ Hook bypass is INACTIVE (hooks run normally)"
    fi
}

function is_bypassed() {
    if [ -f "$BYPASS_FILE" ]; then
        return 0  # true - bypassed
    else
        return 1  # false - not bypassed
    fi
}

# Main command handler
case "${1:-status}" in
    on|enable)
        enable_bypass
        ;;
    off|disable)
        disable_bypass
        ;;
    status)
        status
        ;;
    check)
        # Silent check for use in hooks
        is_bypassed
        exit $?
        ;;
    *)
        echo "Claude Code Hook Bypass Utility"
        echo ""
        echo "Usage: $0 {on|off|status|check}"
        echo ""
        echo "Commands:"
        echo "  on/enable   - Enable hook bypass (create bypass flag)"
        echo "  off/disable - Disable hook bypass (remove bypass flag)"
        echo "  status      - Show current bypass status"
        echo "  check       - Silent check (exit 0 if bypassed, 1 if not)"
        echo ""
        echo "Example hook usage:"
        echo "  if ./scripts/hook-bypass.sh check; then"
        echo "    echo 'Hooks bypassed for testing'"
        echo "    exit 0"
        echo "  fi"
        exit 1
        ;;
esac
