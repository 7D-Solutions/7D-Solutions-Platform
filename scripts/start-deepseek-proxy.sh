#!/bin/bash
# Start DeepSeek Auto-Compact Proxy
# This proxy intercepts API calls and forces compaction at 70% context usage

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROXY_SCRIPT="$SCRIPT_DIR/deepseek-compact-proxy.py"
PID_FILE="/tmp/deepseek-proxy.pid"
LOG_FILE="/tmp/deepseek-proxy.log"

# Configuration
PROXY_PORT="${PROXY_PORT:-5000}"
PROXY_HOST="${PROXY_HOST:-127.0.0.1}"

stop_proxy() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            echo "Stopping proxy (PID $PID)..."
            kill "$PID" 2>/dev/null || true
            sleep 1
            # Force kill if still running
            if ps -p "$PID" > /dev/null 2>&1; then
                kill -9 "$PID" 2>/dev/null || true
            fi
        fi
        rm -f "$PID_FILE"
    fi
}

start_proxy() {
    # Check if already running
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            echo "✓ Proxy already running (PID $PID)"
            echo "  Status: http://127.0.0.1:$PROXY_PORT/status"
            return 0
        else
            rm -f "$PID_FILE"
        fi
    fi

    # Check if Python is available
    if ! command -v python3 &> /dev/null; then
        echo "Error: python3 not found"
        exit 1
    fi

    # Check if Flask is installed
    if ! python3 -c "import flask" 2>/dev/null; then
        echo "Installing Flask..."
        pip3 install flask requests --break-system-packages --quiet || pip3 install flask requests --quiet
    fi

    # Start proxy in background
    echo "Starting DeepSeek Auto-Compact Proxy..."
    echo "  Port: $PROXY_PORT"
    echo "  Log: $LOG_FILE"
    echo ""

    export PROXY_PORT PROXY_HOST
    nohup python3 "$PROXY_SCRIPT" > "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"

    # Wait for startup
    sleep 2

    # Verify it started
    if ps -p $(cat "$PID_FILE") > /dev/null 2>&1; then
        echo "✓ Proxy started successfully (PID $(cat "$PID_FILE"))"
        echo ""
        echo "Status: http://127.0.0.1:$PROXY_PORT/status"
        echo "Health: http://127.0.0.1:$PROXY_PORT/health"
        echo ""
        echo "Tail logs: tail -f $LOG_FILE"
    else
        echo "Error: Proxy failed to start"
        echo "Check logs: cat $LOG_FILE"
        exit 1
    fi
}

status_proxy() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            echo "✓ Proxy running (PID $PID)"
            echo ""
            if command -v curl &> /dev/null; then
                echo "Status:"
                curl -s http://127.0.0.1:$PROXY_PORT/status 2>/dev/null | python3 -m json.tool || echo "  (proxy not responding)"
            fi
            return 0
        else
            echo "✗ Proxy not running (stale PID file)"
            rm -f "$PID_FILE"
            return 1
        fi
    else
        echo "✗ Proxy not running"
        return 1
    fi
}

case "${1:-start}" in
    start)
        start_proxy
        ;;
    stop)
        stop_proxy
        echo "✓ Proxy stopped"
        ;;
    restart)
        stop_proxy
        sleep 1
        start_proxy
        ;;
    status)
        status_proxy
        ;;
    logs)
        if [ -f "$LOG_FILE" ]; then
            tail -f "$LOG_FILE"
        else
            echo "No log file found"
        fi
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status|logs}"
        exit 1
        ;;
esac
