#!/bin/bash
# Grok via Claude Code
# Uses claude-adapter to translate Anthropic API format → OpenAI format for xAI
#
# Claude Code speaks Anthropic format (/v1/messages)
# xAI speaks OpenAI format (/v1/chat/completions)
# claude-adapter bridges the two as a local proxy
#
# Usage: ./scripts/grok-claude-wrapper.sh [model]
#   model defaults to grok-3-mini-beta (reasoning, slower but cheaper)
#   alternatives: grok-3-beta (fast, no reasoning), grok-4-latest (reasoning, expensive)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

if [ -z "${XAI_API_KEY:-}" ]; then
    echo "Error: XAI_API_KEY environment variable not set"
    echo "Please run: export XAI_API_KEY='your-api-key'"
    exit 1
fi

if ! command -v claude-adapter &> /dev/null; then
    echo "Error: claude-adapter not installed"
    echo "Install with: npm install -g claude-adapter"
    exit 1
fi

# Model selection
GROK_MODEL="${1:-grok-3-mini-beta}"
shift 2>/dev/null || true

# Pick a port for this adapter instance
# Use a hash of the tmux pane to get a stable per-pane port, or default
ADAPTER_PORT="${GROK_ADAPTER_PORT:-3090}"
if [ -n "${TMUX:-}" ]; then
    TMUX_PANE=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null || echo "")
    if [ -n "$TMUX_PANE" ]; then
        # Generate a stable port from pane identity (range 3090-3190)
        PANE_HASH=$(echo "$TMUX_PANE" | cksum | awk '{print $1}')
        ADAPTER_PORT=$(( 3090 + (PANE_HASH % 100) ))
    fi
fi

# Register with mail system — clear stale identity and get fresh name
MAIL_PROJECT_KEY="${MAIL_PROJECT_KEY:-$PROJECT_ROOT}"
source "$SCRIPT_DIR/lib/pane-init.sh"
init_pane "Grok Agent"

# Write claude-adapter config for xAI
ADAPTER_CONFIG_DIR="$PROJECT_ROOT/.grok-adapter"
mkdir -p "$ADAPTER_CONFIG_DIR"
ADAPTER_CONFIG="$ADAPTER_CONFIG_DIR/config-${ADAPTER_PORT}.json"
cat > "$ADAPTER_CONFIG" << EOF
{
  "baseUrl": "https://api.x.ai/v1",
  "apiKey": "${XAI_API_KEY}",
  "models": {
    "opus": "${GROK_MODEL}",
    "sonnet": "${GROK_MODEL}",
    "haiku": "${GROK_MODEL}"
  },
  "toolFormat": "native"
}
EOF

# Start claude-adapter in background (won't touch global settings)
# Check if adapter is already running on this port
ADAPTER_PID=""
if curl -sf "http://localhost:${ADAPTER_PORT}/health" > /dev/null 2>&1; then
    echo "claude-adapter already running on port ${ADAPTER_PORT}"
else
    echo "Starting claude-adapter on port ${ADAPTER_PORT}..."

    # Copy config to where claude-adapter expects it
    mkdir -p ~/.claude-adapter
    cp "$ADAPTER_CONFIG" ~/.claude-adapter/config.json

    # Use local patched adapter if available, otherwise fall back to global
    PATCHED_ADAPTER="$PROJECT_ROOT/.grok-adapter/claude-adapter-patched/dist/cli.js"
    if [ -f "$PATCHED_ADAPTER" ]; then
        echo "Using patched claude-adapter"
        node "$PATCHED_ADAPTER" --port "$ADAPTER_PORT" --no-claude-settings > "$ADAPTER_CONFIG_DIR/adapter-${ADAPTER_PORT}.log" 2>&1 &
    else
        claude-adapter --port "$ADAPTER_PORT" --no-claude-settings > "$ADAPTER_CONFIG_DIR/adapter-${ADAPTER_PORT}.log" 2>&1 &
    fi
    ADAPTER_PID=$!

    # Wait for adapter to be ready
    for i in $(seq 1 15); do
        if curl -sf "http://localhost:${ADAPTER_PORT}/health" > /dev/null 2>&1; then
            echo "claude-adapter ready on port ${ADAPTER_PORT}"
            break
        fi
        if ! kill -0 "$ADAPTER_PID" 2>/dev/null; then
            echo "Error: claude-adapter failed to start. Check $ADAPTER_CONFIG_DIR/adapter-${ADAPTER_PORT}.log"
            exit 1
        fi
        sleep 1
    done

    if ! curl -sf "http://localhost:${ADAPTER_PORT}/health" > /dev/null 2>&1; then
        echo "Error: claude-adapter didn't become ready in time"
        kill "$ADAPTER_PID" 2>/dev/null || true
        exit 1
    fi
fi

# Clean up adapter on exit
cleanup() {
    if [ -n "$ADAPTER_PID" ]; then
        kill "$ADAPTER_PID" 2>/dev/null || true
    fi
    # Restore original claude-adapter config if we backed it up
    if [ -f ~/.claude-adapter/config.json.grok-backup ]; then
        mv ~/.claude-adapter/config.json.grok-backup ~/.claude-adapter/config.json
    fi
}
trap cleanup EXIT INT TERM

# Back up existing claude-adapter config if it wasn't ours
if [ -f ~/.claude-adapter/config.json ] && [ -z "$ADAPTER_PID" ]; then
    # We didn't start the adapter, don't mess with the config
    true
fi

# Clear any existing Anthropic credentials
unset ANTHROPIC_API_KEY
unset ANTHROPIC_BASE_URL
unset ANTHROPIC_MODEL
unset ANTHROPIC_SMALL_FAST_MODEL

# Point Claude Code at the local adapter (NOT directly at xAI)
export ANTHROPIC_BASE_URL="http://localhost:${ADAPTER_PORT}"
export ANTHROPIC_AUTH_TOKEN="default"
export ANTHROPIC_MODEL="$GROK_MODEL"
export ANTHROPIC_SMALL_FAST_MODEL="${GROK_FAST_MODEL:-grok-3-beta}"
export API_TIMEOUT_MS=600000
export CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1

# Export for mail helper
export AGENT_NAME
export PROJECT_ROOT
export MAIL_PROJECT_KEY

# Launch Claude Code
echo "====================================="
echo "Grok Agent: $AGENT_NAME"
echo "====================================="
echo "Project: $PROJECT_ROOT"
echo "Model: $GROK_MODEL"
echo "Adapter: localhost:${ADAPTER_PORT} → api.x.ai"
echo "====================================="
echo ""

exec claude --dangerously-skip-permissions \
    --system-prompt "You are $AGENT_NAME, a Grok agent (model: $GROK_MODEL) in a multi-agent tmux environment.
Follow the instructions in CLAUDE.md for mail, beads, and coordination.

IMPORTANT: Respond directly to simple messages (greetings, short questions, clarifications) without launching Task agents. Only use Task agents for genuinely complex multi-step work. The 'greeting-responder' in the tool docs is just an example, not a real agent." \
    "$@"
