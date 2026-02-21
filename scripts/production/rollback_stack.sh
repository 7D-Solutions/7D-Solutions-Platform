#!/usr/bin/env bash
# rollback_stack.sh — Roll back production to a prior immutable image tag.
#
# Rollback = deploy a previously-pushed tag.  All images for that tag must
# already exist in the registry (they were published by the original release).
#
# Usage:
#   # Roll back to an explicit tag:
#   bash scripts/production/rollback_stack.sh --tag v1.0.0-abc1234
#
#   # Roll back to the tag before the current one (reads .production-deployments log):
#   bash scripts/production/rollback_stack.sh --previous
#
#   # Show recent deployment history on VPS:
#   bash scripts/production/rollback_stack.sh --history
#
# Required environment variables (same as deploy_stack.sh):
#   PROD_HOST, PROD_USER, PROD_REPO_PATH, IMAGE_REGISTRY

set -euo pipefail

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
TAG=""
PREVIOUS=false
SHOW_HISTORY=false
PASSTHROUGH_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag)         TAG="$2"; shift 2 ;;
        --previous)    PREVIOUS=true; shift ;;
        --history)     SHOW_HISTORY=true; shift ;;
        --dry-run|--skip-smoke) PASSTHROUGH_ARGS+=("$1"); shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Connection config
# ---------------------------------------------------------------------------
PROD_HOST="${PROD_HOST:?ERROR: PROD_HOST must be set}"
PROD_USER="${PROD_USER:-deploy}"
PROD_REPO_PATH="${PROD_REPO_PATH:-/opt/7d-platform}"
PROD_SSH_PORT="${PROD_SSH_PORT:-22}"

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${PROD_SSH_PORT}"
SSH_TARGET="${PROD_USER}@${PROD_HOST}"
DEPLOY_LOG="${PROD_REPO_PATH}/.production-deployments"

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[rollback_stack:prod] $*"; }

run_remote() {
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" "$*"
}

# ---------------------------------------------------------------------------
# --history: show deployment log and exit
# ---------------------------------------------------------------------------
if $SHOW_HISTORY; then
    banner "Production deployment history"
    run_remote "cat ${DEPLOY_LOG} 2>/dev/null || echo '(no deployments recorded)'"
    exit 0
fi

# ---------------------------------------------------------------------------
# --previous: resolve prior tag from deployment log
# ---------------------------------------------------------------------------
if $PREVIOUS; then
    banner "Resolving previous tag from deployment log"
    RAW_LOG="$(run_remote "cat ${DEPLOY_LOG} 2>/dev/null || true")"

    if [[ -z "$RAW_LOG" ]]; then
        echo "ERROR: Deployment log is empty. Cannot determine previous tag." >&2
        exit 1
    fi

    # Log lines: "2026-02-21T12:00:00Z tag=v1.0.0-abc1234 registry=7dsolutions"
    # Current = last line. Previous = second-to-last.
    LINE_COUNT=$(echo "$RAW_LOG" | wc -l | tr -d ' ')
    if [[ "$LINE_COUNT" -lt 2 ]]; then
        echo "ERROR: Only one deployment in the log. No prior tag to roll back to." >&2
        echo ""
        echo "Deployment history:"
        echo "$RAW_LOG"
        exit 1
    fi

    PREV_LINE="$(echo "$RAW_LOG" | tail -2 | head -1)"
    TAG="$(echo "$PREV_LINE" | grep -oP '(?<=tag=)\S+')"

    if [[ -z "$TAG" ]]; then
        echo "ERROR: Could not parse tag from log line: $PREV_LINE" >&2
        echo "       (Manifest-mode deploys do not record a single tag — use --tag explicitly.)" >&2
        exit 1
    fi

    log "Resolved previous tag: ${TAG}"
fi

# ---------------------------------------------------------------------------
# Require --tag at this point
# ---------------------------------------------------------------------------
if [[ -z "$TAG" ]]; then
    echo "ERROR: Specify a tag with --tag <tag> or use --previous." >&2
    echo "       Use --history to see recent deployments." >&2
    exit 1
fi

banner "Rolling back production to ${TAG}"
log "This re-runs deploy_stack.sh with the prior tag."
log "All images for tag=${TAG} must already exist in the registry."
echo ""

# ---------------------------------------------------------------------------
# Delegate to deploy_stack.sh
# ---------------------------------------------------------------------------
DEPLOY_SCRIPT="$(dirname "$0")/deploy_stack.sh"
if [[ ! -f "$DEPLOY_SCRIPT" ]]; then
    echo "ERROR: deploy_stack.sh not found at: $DEPLOY_SCRIPT" >&2
    exit 1
fi

exec bash "$DEPLOY_SCRIPT" --tag "$TAG" "${PASSTHROUGH_ARGS[@]}"
