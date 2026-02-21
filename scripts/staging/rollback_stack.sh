#!/usr/bin/env bash
# rollback_stack.sh — Roll back staging by selecting a prior manifest snapshot.
#
# Rollback = restore an older MODULE-MANIFEST.md and re-deploy. Since the
# manifest IS the single source of truth for image tags, rolling back means
# deploying exactly what that prior manifest declared.
#
# Two resolution modes:
#   --previous          Use the VPS manifest snapshot taken just before the
#                       most recent deploy (from the .manifest-snapshots archive).
#   --snapshot <name>   Use a specific snapshot by filename (see --history).
#   --manifest-sha <sha>  Extract manifest at a prior git commit on this repo.
#
# Show deployment history:
#   --history           Print deployment log and available manifest snapshots.
#
# Usage:
#   bash scripts/staging/rollback_stack.sh --previous
#   bash scripts/staging/rollback_stack.sh --previous --dry-run
#   bash scripts/staging/rollback_stack.sh --snapshot 2026-02-20T14:00:00Z-MODULE-MANIFEST.md
#   bash scripts/staging/rollback_stack.sh --manifest-sha abc1234
#   bash scripts/staging/rollback_stack.sh --history
#
# Required environment variables (same as deploy_stack.sh):
#   STAGING_HOST, STAGING_USER, STAGING_REPO_PATH, IMAGE_REGISTRY

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_FILE_REL="deploy/staging/MODULE-MANIFEST.md"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
MODE=""
SNAPSHOT_NAME=""
MANIFEST_GIT_SHA=""
PASSTHROUGH_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --previous)      MODE="previous"; shift ;;
        --snapshot)      MODE="snapshot"; SNAPSHOT_NAME="$2"; shift 2 ;;
        --manifest-sha)  MODE="git-sha"; MANIFEST_GIT_SHA="$2"; shift 2 ;;
        --history)       MODE="history"; shift ;;
        --dry-run|--skip-smoke) PASSTHROUGH_ARGS+=("$1"); shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$MODE" ]]; then
    echo "ERROR: Specify a rollback mode." >&2
    echo "  --previous             Roll back to the snapshot before the last deploy." >&2
    echo "  --snapshot <name>      Roll back to a specific named snapshot." >&2
    echo "  --manifest-sha <sha>   Roll back to manifest at a prior git commit." >&2
    echo "  --history              Show deployment log and available snapshots." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Connection config
# ---------------------------------------------------------------------------
STAGING_HOST="${STAGING_HOST:?ERROR: STAGING_HOST must be set}"
STAGING_USER="${STAGING_USER:-deploy}"
STAGING_REPO_PATH="${STAGING_REPO_PATH:-/opt/7d-platform}"
STAGING_SSH_PORT="${STAGING_SSH_PORT:-22}"

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${STAGING_SSH_PORT}"
SSH_TARGET="${STAGING_USER}@${STAGING_HOST}"
DEPLOY_LOG="${STAGING_REPO_PATH}/.staging-deployments"
SNAPSHOT_DIR="${STAGING_REPO_PATH}/.manifest-snapshots"

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[rollback_stack] $*"; }

run_remote() {
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" "$*"
}

# ---------------------------------------------------------------------------
# --history: show deployment log and available snapshots, then exit
# ---------------------------------------------------------------------------
if [[ "$MODE" == "history" ]]; then
    banner "Staging deployment history"
    run_remote "cat ${DEPLOY_LOG} 2>/dev/null || echo '(no deployments recorded)'"

    banner "Available manifest snapshots (${SNAPSHOT_DIR})"
    run_remote "ls -1t ${SNAPSHOT_DIR}/ 2>/dev/null | head -20 || echo '(no snapshots found)'"
    exit 0
fi

# ---------------------------------------------------------------------------
# Resolve the rollback manifest to a local temp file
# ---------------------------------------------------------------------------
TEMP_MANIFEST="$(mktemp /tmp/rollback-manifest-XXXXXX.md)"
trap 'rm -f "$TEMP_MANIFEST"' EXIT

if [[ "$MODE" == "previous" ]]; then
    banner "Resolving previous manifest snapshot from VPS"

    # List snapshots sorted by time (newest first), pick the second one
    SNAPSHOT_LIST="$(run_remote "ls -1t ${SNAPSHOT_DIR}/ 2>/dev/null || true")"
    if [[ -z "$SNAPSHOT_LIST" ]]; then
        echo "ERROR: No manifest snapshots found on VPS at ${SNAPSHOT_DIR}." >&2
        echo "       Ensure at least two deploys have been run with this script." >&2
        exit 1
    fi

    SNAPSHOT_COUNT=$(echo "$SNAPSHOT_LIST" | wc -l | tr -d ' ')
    if [[ "$SNAPSHOT_COUNT" -lt 2 ]]; then
        echo "ERROR: Only one manifest snapshot exists. No prior state to roll back to." >&2
        echo ""
        echo "Available snapshot:"
        echo "$SNAPSHOT_LIST"
        exit 1
    fi

    # Second entry = the snapshot before the most recent deploy
    SNAPSHOT_NAME="$(echo "$SNAPSHOT_LIST" | sed -n '2p')"
    log "Selected prior snapshot: ${SNAPSHOT_NAME}"

    scp -q -P "${STAGING_SSH_PORT}" -o StrictHostKeyChecking=no \
        "${SSH_TARGET}:${SNAPSHOT_DIR}/${SNAPSHOT_NAME}" "$TEMP_MANIFEST"
    log "Downloaded snapshot to: ${TEMP_MANIFEST}"

elif [[ "$MODE" == "snapshot" ]]; then
    banner "Resolving named manifest snapshot: ${SNAPSHOT_NAME}"
    scp -q -P "${STAGING_SSH_PORT}" -o StrictHostKeyChecking=no \
        "${SSH_TARGET}:${SNAPSHOT_DIR}/${SNAPSHOT_NAME}" "$TEMP_MANIFEST"
    log "Downloaded snapshot to: ${TEMP_MANIFEST}"

elif [[ "$MODE" == "git-sha" ]]; then
    banner "Extracting manifest at git commit: ${MANIFEST_GIT_SHA}"

    if ! git -C "$REPO_ROOT" cat-file -e "${MANIFEST_GIT_SHA}" 2>/dev/null; then
        echo "ERROR: Git SHA not found in repo: ${MANIFEST_GIT_SHA}" >&2
        exit 1
    fi

    git -C "$REPO_ROOT" show "${MANIFEST_GIT_SHA}:${MANIFEST_FILE_REL}" > "$TEMP_MANIFEST" 2>/dev/null || {
        # Try the SHA as a commit ref: get the manifest AT that commit
        git -C "$REPO_ROOT" show "${MANIFEST_GIT_SHA}^{commit}:${MANIFEST_FILE_REL}" > "$TEMP_MANIFEST"
    }
    log "Extracted manifest from git ${MANIFEST_GIT_SHA} to: ${TEMP_MANIFEST}"
fi

# Sanity check: temp manifest has content
if [[ ! -s "$TEMP_MANIFEST" ]]; then
    echo "ERROR: Resolved manifest is empty: ${TEMP_MANIFEST}" >&2
    exit 1
fi

banner "Rolling back staging to: ${SNAPSHOT_NAME:-${MANIFEST_GIT_SHA}}"
echo ""
echo "Manifest to deploy:"
grep '^|' "$TEMP_MANIFEST" | grep -v '^|---' | head -20
echo ""
echo "All images for these versions must already exist in the registry."
echo "Rollback is a full re-deploy using the selected manifest."
echo ""

# ---------------------------------------------------------------------------
# Gate 3 check: pass manifest through validate script before deploying
# ---------------------------------------------------------------------------
VALIDATE_SCRIPT="${REPO_ROOT}/scripts/staging/manifest_validate.sh"
if [[ -f "$VALIDATE_SCRIPT" ]]; then
    banner "Running manifest validation (Gate 3 pre-check)"
    MANIFEST_FILE="$TEMP_MANIFEST" bash "$VALIDATE_SCRIPT" || {
        echo ""
        echo "ERROR: Manifest validation failed. Images for this manifest may not exist in" >&2
        echo "       the registry. Verify with 'docker manifest inspect <image>' before rollback." >&2
        exit 1
    }
fi

# ---------------------------------------------------------------------------
# Delegate to deploy_stack.sh with the resolved manifest
# ---------------------------------------------------------------------------
DEPLOY_SCRIPT="$(dirname "$0")/deploy_stack.sh"
if [[ ! -f "$DEPLOY_SCRIPT" ]]; then
    echo "ERROR: deploy_stack.sh not found at: $DEPLOY_SCRIPT" >&2
    exit 1
fi

exec bash "$DEPLOY_SCRIPT" --manifest "$TEMP_MANIFEST" "${PASSTHROUGH_ARGS[@]}"
