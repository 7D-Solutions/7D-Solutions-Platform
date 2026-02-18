#!/bin/bash
# Cargo Build Slot System — allows 2 concurrent cargo operations
# Usage: ./scripts/cargo-slot.sh test -p inventory-rs
#        ./scripts/cargo-slot.sh build --release
#        ./scripts/cargo-slot.sh --warm    # pre-warm both slots
#
# Agents use this instead of raw `cargo` to avoid build lock contention.
# Two slots with independent CARGO_TARGET_DIR directories.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Resolve the real cargo binary, bypassing the workspace bin/ shim.
# The bin/cargo shim calls this script — do NOT use `cargo` directly or we recurse.
# Try known locations in priority order, skipping the workspace shim.
REAL_CARGO=""
for _candidate in \
    "$HOME/.cargo/bin/cargo" \
    /usr/local/bin/cargo \
    /opt/homebrew/bin/cargo \
    /usr/bin/cargo; do
    if [ -x "$_candidate" ]; then
        REAL_CARGO="$_candidate"
        break
    fi
done
if [ -z "$REAL_CARGO" ]; then
    echo "[cargo-slot] ERROR: Cannot find real cargo binary (tried ~/.cargo/bin, /usr/local/bin, etc.)." >&2
    exit 1
fi

# Workspace-unique lock root (hash of workspace path)
WORKSPACE_HASH=$(echo -n "$WORKSPACE_ROOT" | shasum -a 256 | cut -c1-12)
LOCK_ROOT="/tmp/cargo-build-slots/$WORKSPACE_HASH"
SLOT_COUNT=2

# Target dirs live in workspace root (gitignored)
slot_target_dir() {
    echo "$WORKSPACE_ROOT/target-slot-$1"
}

slot_lock_dir() {
    echo "$LOCK_ROOT/slot-$1"
}

slot_pid_file() {
    echo "$(slot_lock_dir "$1")/pid"
}

# Check if a PID is alive and is a cargo-related process
is_holder_alive() {
    local pid="$1"
    if [ -z "$pid" ]; then return 1; fi
    if ! kill -0 "$pid" 2>/dev/null; then return 1; fi
    # Verify it's a cargo/rustc descendant (not a recycled PID)
    local cmd
    cmd=$(ps -p "$pid" -o comm= 2>/dev/null || echo "")
    case "$cmd" in
        cargo*|rustc*|cc*|ld*|bash*|zsh*|sh*|node*|claude*) return 0 ;;
        *) return 1 ;;
    esac
}

acquire_slot() {
    mkdir -p "$LOCK_ROOT"
    for i in $(seq 1 $SLOT_COUNT); do
        local lockdir
        lockdir="$(slot_lock_dir "$i")"
        if mkdir "$lockdir" 2>/dev/null; then
            echo $$ > "$(slot_pid_file "$i")"
            echo "$i"
            return 0
        fi
        # Check for stale lock
        local pidfile
        pidfile="$(slot_pid_file "$i")"
        if [ -f "$pidfile" ]; then
            local held_pid
            held_pid=$(cat "$pidfile" 2>/dev/null || echo "")
            if ! is_holder_alive "$held_pid"; then
                echo "Reclaiming stale slot $i (dead PID $held_pid)" >&2
                rm -rf "$lockdir"
                if mkdir "$lockdir" 2>/dev/null; then
                    echo $$ > "$(slot_pid_file "$i")"
                    echo "$i"
                    return 0
                fi
            fi
        fi
    done
    return 1
}

release_slot() {
    local slot="$1"
    rm -rf "$(slot_lock_dir "$slot")" 2>/dev/null
}

show_status() {
    echo "=== Cargo Build Slots ==="
    echo "Workspace: $WORKSPACE_ROOT"
    echo "Lock root: $LOCK_ROOT"
    echo ""
    for i in $(seq 1 $SLOT_COUNT); do
        local lockdir pidfile status holder
        lockdir="$(slot_lock_dir "$i")"
        pidfile="$(slot_pid_file "$i")"
        if [ -d "$lockdir" ] && [ -f "$pidfile" ]; then
            holder=$(cat "$pidfile" 2>/dev/null || echo "?")
            if is_holder_alive "$holder"; then
                local cmd
                cmd=$(ps -p "$holder" -o command= 2>/dev/null | head -c 60 || echo "unknown")
                status="BUSY (PID $holder: $cmd)"
            else
                status="STALE (dead PID $holder)"
            fi
        else
            status="FREE"
        fi
        local tdir
        tdir="$(slot_target_dir "$i")"
        local size="(empty)"
        if [ -d "$tdir" ]; then
            size="$(du -sh "$tdir" 2>/dev/null | cut -f1 || echo "?")"
        fi
        echo "  Slot $i: $status  [$size]"
    done
}

warm_slots() {
    echo "Pre-warming $SLOT_COUNT cargo slots..." >&2
    for i in $(seq 1 $SLOT_COUNT); do
        local tdir
        tdir="$(slot_target_dir "$i")"
        echo "  Warming slot $i → $tdir" >&2
        CARGO_TARGET_DIR="$tdir" "$REAL_CARGO" build -p inventory-rs -q 2>&1
        echo "  Slot $i warm." >&2
    done
    echo "All slots warmed." >&2
}

# Handle special commands
case "${1:-}" in
    --status)
        show_status
        exit 0
        ;;
    --warm)
        warm_slots
        exit 0
        ;;
    --clean)
        echo "Cleaning all slot locks..." >&2
        rm -rf "$LOCK_ROOT"
        echo "Done. Target dirs preserved (delete target-slot-*/ manually if needed)." >&2
        exit 0
        ;;
    "")
        echo "Usage: cargo-slot.sh <cargo-subcommand> [args...]" >&2
        echo "       cargo-slot.sh --status   Show slot status" >&2
        echo "       cargo-slot.sh --warm     Pre-warm both slots" >&2
        echo "       cargo-slot.sh --clean    Remove all locks" >&2
        exit 1
        ;;
esac

# Acquire a slot
SLOT=""
WAIT_COUNT=0
while [ -z "$SLOT" ]; do
    SLOT=$(acquire_slot || echo "")
    if [ -z "$SLOT" ]; then
        if [ "$WAIT_COUNT" -eq 0 ]; then
            echo "All cargo slots busy, waiting..." >&2
        fi
        WAIT_COUNT=$((WAIT_COUNT + 1))
        if [ "$((WAIT_COUNT % 15))" -eq 0 ]; then
            echo "Still waiting for a cargo slot (${WAIT_COUNT}s)..." >&2
        fi
        sleep 2
    fi
done

# Cleanup on exit
trap "release_slot $SLOT" EXIT INT TERM HUP

TARGET_DIR="$(slot_target_dir "$SLOT")"
export CARGO_TARGET_DIR="$TARGET_DIR"

echo "[cargo-slot] Using slot $SLOT → $TARGET_DIR" >&2

# Run cargo with all arguments (use resolved binary to avoid calling the bin/ shim recursively)
cd "$WORKSPACE_ROOT"
"$REAL_CARGO" "$@"
