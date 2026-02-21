#!/usr/bin/env bash
# detect_version_intent.sh — Detect whether a commit range contains a version-intent change.
#
# A "version-intent change" is any diff that:
#   - Bumps the version field in a module/platform Cargo.toml ([package] section)
#   - Bumps the "version" field in an apps/* package.json
#   - Changes any MODULE-MANIFEST.md file
#
# Usage:
#   bash scripts/versioning/detect_version_intent.sh [BASE] [HEAD]
#
#   BASE defaults to HEAD~1
#   HEAD defaults to HEAD
#
# Exit codes:
#   0 — version intent detected (triggering files printed to stdout)
#   1 — no version intent detected
#   2 — error (bad args, git failure)

set -euo pipefail

BASE="${1:-HEAD~1}"
HEAD_REF="${2:-HEAD}"

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

log() { echo "[detect_version_intent] $*" >&2; }

# Validate that the base commit is reachable
if ! git rev-parse --verify "$BASE" >/dev/null 2>&1; then
    log "ERROR: Base commit not reachable: $BASE"
    exit 2
fi

# Get changed files between BASE and HEAD
changed_files="$(git diff --name-only "$BASE" "$HEAD_REF" 2>/dev/null)" || {
    log "ERROR: Could not diff $BASE..$HEAD_REF"
    exit 2
}

if [ -z "$changed_files" ]; then
    log "No changed files between $BASE and $HEAD_REF"
    exit 1
fi

DETECTED=()

while IFS= read -r file; do
    case "$file" in
        # Module or platform Cargo.toml (not the workspace root Cargo.toml)
        modules/*/Cargo.toml | platform/*/Cargo.toml)
            diff_out="$(git diff "$BASE" "$HEAD_REF" -- "$file" 2>/dev/null)"
            if echo "$diff_out" | grep -qE '^\+version = "[0-9]'; then
                log "Version intent: $file (Cargo.toml version bump)"
                DETECTED+=("$file")
            fi
            ;;

        # App package.json — detect "version" field bump
        apps/*/package.json)
            diff_out="$(git diff "$BASE" "$HEAD_REF" -- "$file" 2>/dev/null)"
            if echo "$diff_out" | grep -qE '^\+[[:space:]]*"version":[[:space:]]*"[0-9]'; then
                log "Version intent: $file (package.json version bump)"
                DETECTED+=("$file")
            fi
            ;;

        # MODULE-MANIFEST.md — any change signals version intent
        *MODULE-MANIFEST.md)
            log "Version intent: $file (MODULE-MANIFEST changed)"
            DETECTED+=("$file")
            ;;
    esac
done <<< "$changed_files"

if [ "${#DETECTED[@]}" -gt 0 ]; then
    echo "VERSION_INTENT_FILES:"
    for f in "${DETECTED[@]}"; do
        echo "  $f"
    done
    exit 0
fi

log "No version intent detected"
exit 1
