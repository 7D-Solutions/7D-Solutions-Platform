#!/usr/bin/env bash
# wait-for-ready.sh — Poll /api/ready for one or more module endpoints.
#
# Usage:
#   wait-for-ready.sh [OPTIONS] <base_url> [<base_url> ...]
#
# Options:
#   --tenant-id <uuid>   Also poll /api/ready?tenant_id=<uuid> and require
#                        tenant.status == "up" before marking ready.
#   --timeout <seconds>  Overall timeout in seconds (default: 120).
#   --interval <seconds> Poll interval in seconds (default: 2).
#   --quiet              Suppress per-attempt output; only print final result.
#
# Exit codes:
#   0  All endpoints returned status="ready" (and tenant status="up" if requested).
#   1  Timeout reached before all endpoints were ready.
#
# Examples:
#   # Wait for two modules to be globally ready:
#   wait-for-ready.sh http://localhost:8080 http://localhost:8081
#
#   # Wait for tenant abc-123 to be provisioned in all modules:
#   wait-for-ready.sh --tenant-id abc-123 http://localhost:8080 http://localhost:8081

set -euo pipefail

TIMEOUT=120
INTERVAL=2
TENANT_ID=""
QUIET=0
URLS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --tenant-id)
            TENANT_ID="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --interval)
            INTERVAL="$2"
            shift 2
            ;;
        --quiet)
            QUIET=1
            shift
            ;;
        -*)
            echo "unknown option: $1" >&2
            exit 1
            ;;
        *)
            URLS+=("$1")
            shift
            ;;
    esac
done

if [[ ${#URLS[@]} -eq 0 ]]; then
    echo "usage: wait-for-ready.sh [--tenant-id <uuid>] [--timeout N] [--interval N] <base_url>..." >&2
    exit 1
fi

# Build the path suffix for each poll
if [[ -n "$TENANT_ID" ]]; then
    READY_PATH="/api/ready?tenant_id=${TENANT_ID}"
else
    READY_PATH="/api/ready"
fi

deadline=$(( $(date +%s) + TIMEOUT ))

is_ready() {
    local url="$1${READY_PATH}"
    local response
    response=$(curl --silent --max-time 5 "$url" 2>/dev/null) || return 1
    local status
    status=$(echo "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('status',''))" 2>/dev/null) || return 1
    [[ "$status" == "ready" ]] || return 1

    if [[ -n "$TENANT_ID" ]]; then
        local tenant_status
        tenant_status=$(echo "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tenant',{}).get('status',''))" 2>/dev/null) || return 1
        [[ "$tenant_status" == "up" ]] || return 1
    fi
    return 0
}

while true; do
    all_ready=1
    for base in "${URLS[@]}"; do
        if is_ready "$base"; then
            [[ $QUIET -eq 0 ]] && echo "$(date -u +%H:%M:%SZ) $base${READY_PATH} ready"
        else
            [[ $QUIET -eq 0 ]] && echo "$(date -u +%H:%M:%SZ) $base${READY_PATH} waiting..."
            all_ready=0
        fi
    done

    if [[ $all_ready -eq 1 ]]; then
        echo "all endpoints ready"
        exit 0
    fi

    now=$(date +%s)
    if [[ $now -ge $deadline ]]; then
        echo "timeout after ${TIMEOUT}s — not all endpoints reached ready" >&2
        exit 1
    fi

    sleep "$INTERVAL"
done
