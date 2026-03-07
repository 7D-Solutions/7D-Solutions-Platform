#!/usr/bin/env bash
# staging_push.sh — Build, tag, and push Docker images to GHCR for staging.
#
# Usage:
#   ./scripts/staging_push.sh            # Build and push
#   ./scripts/staging_push.sh --dry-run  # Show what would happen
#
# Requires: docker login to ghcr.io (see docs/DEPLOYMENT.md)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

REGISTRY="ghcr.io/7d-solutions"
DRY_RUN=false

if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=true
fi

# --- Service definitions ---
# Each entry: name|version_file|version_cmd|dockerfile|build_context
#   version_file: path to Cargo.toml or package.json
#   version_cmd: command to extract version from the file
#   dockerfile: path to Dockerfile
#   build_context: Docker build context directory

SERVICES=(
    "auth|platform/identity-auth/Cargo.toml|toml|platform/identity-auth/Dockerfile.workspace|."
    "ar|modules/ar/Cargo.toml|toml|modules/ar/deploy/Dockerfile.workspace|."
    "tcp-ui|apps/tenant-control-plane-ui/package.json|json|apps/tenant-control-plane-ui/Dockerfile|apps/tenant-control-plane-ui"
)

# --- Helper functions ---

extract_version_toml() {
    local file="$1"
    grep '^version' "$file" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

extract_version_json() {
    local file="$1"
    grep '"version"' "$file" | head -1 | sed 's/.*"\([0-9][0-9.]*\)".*/\1/'
}

extract_version() {
    local file="$1"
    local format="$2"
    case "$format" in
        toml) extract_version_toml "$file" ;;
        json) extract_version_json "$file" ;;
        *) echo "ERROR: Unknown format $format" >&2; exit 1 ;;
    esac
}

log() {
    echo "[staging_push] $*"
}

# --- Main ---

cd "$PROJECT_ROOT"

log "Registry: $REGISTRY"
log "Mode: $(if $DRY_RUN; then echo 'DRY RUN'; else echo 'LIVE'; fi)"
echo ""

# Validate all versions first
log "=== Version check ==="
declare -A IMAGE_MAP
for entry in "${SERVICES[@]}"; do
    IFS='|' read -r name version_file format dockerfile build_context <<< "$entry"

    if [[ ! -f "$version_file" ]]; then
        echo "ERROR: Version file not found: $version_file" >&2
        exit 1
    fi

    version=$(extract_version "$version_file" "$format")
    if [[ -z "$version" ]]; then
        echo "ERROR: Could not extract version from $version_file" >&2
        exit 1
    fi

    image="${REGISTRY}/7d-${name}:${version}"
    IMAGE_MAP["$name"]="$image"

    log "  $name: v${version} -> $image"
done
echo ""

# Build and push each service
log "=== Build + push ==="
ERRORS=()
for entry in "${SERVICES[@]}"; do
    IFS='|' read -r name version_file format dockerfile build_context <<< "$entry"

    version=$(extract_version "$version_file" "$format")
    image="${REGISTRY}/7d-${name}:${version}"

    if [[ ! -f "$dockerfile" ]]; then
        echo "ERROR: Dockerfile not found: $dockerfile" >&2
        ERRORS+=("$name: missing Dockerfile at $dockerfile")
        continue
    fi

    log "--- $name ---"
    log "  Dockerfile: $dockerfile"
    log "  Context:    $build_context"
    log "  Image:      $image"

    if $DRY_RUN; then
        log "  [dry-run] Would run: docker build -t $image -f $dockerfile $build_context"
        log "  [dry-run] Would run: docker push $image"
    else
        log "  Building..."
        docker build -t "$image" -f "$dockerfile" "$build_context"
        log "  Pushing..."
        docker push "$image"
        log "  Done."
    fi
    echo ""
done

# Summary
log "=== Summary ==="
for entry in "${SERVICES[@]}"; do
    IFS='|' read -r name version_file format dockerfile build_context <<< "$entry"
    version=$(extract_version "$version_file" "$format")
    image="${REGISTRY}/7d-${name}:${version}"
    log "  $name -> $image"
done

if [[ ${#ERRORS[@]} -gt 0 ]]; then
    echo ""
    log "ERRORS:"
    for err in "${ERRORS[@]}"; do
        log "  - $err"
    done
    exit 1
fi

if $DRY_RUN; then
    echo ""
    log "Dry run complete. No images were built or pushed."
fi
