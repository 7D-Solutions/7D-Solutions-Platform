#!/usr/bin/env bash
# push_images.sh — Push built staging images to the container registry.
#
# Safety invariants (all enforced unconditionally):
#   1. Refuses to push any image tagged 'latest'
#   2. Refuses to push if the tag already exists in the registry (warns and aborts)
#   3. Requires --confirm flag before any push occurs
#
# Usage:
#   bash scripts/staging/push_images.sh --confirm          # push all services
#   bash scripts/staging/push_images.sh --confirm ar ttp   # push specific services
#   bash scripts/staging/push_images.sh --dry-run          # print what would be pushed
#
# Environment:
#   IMAGE_REGISTRY  Registry prefix (default: 7dsolutions)
#   DOCKER_USERNAME / DOCKER_PASSWORD  If set, docker login is performed first.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CONFIRM=false
DRY_RUN=false
TARGETS=()

for arg in "$@"; do
    case "$arg" in
        --confirm) CONFIRM=true ;;
        --dry-run) DRY_RUN=true ;;
        --*) echo "Unknown flag: $arg" >&2; exit 1 ;;
        *) TARGETS+=("$arg") ;;
    esac
done

if ! $DRY_RUN && ! $CONFIRM; then
    echo "ERROR: You must pass --confirm to push images." >&2
    echo "       Use --dry-run to preview what would be pushed." >&2
    exit 1
fi

REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

extract_cargo_version() {
    local cargo_toml="$1"
    grep '^version' "$cargo_toml" | head -1 | sed 's/version = "//; s/"//'
}

extract_npm_version() {
    local pkg_json="$1"
    python3 -c "import json; d=json.load(open('$pkg_json')); print(d['version'], end='')"
}

log() { echo "[push_images] $*"; }
run() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] $*"
    else
        "$@"
    fi
}

tag_exists_in_registry() {
    local image="$1"
    docker manifest inspect "$image" >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Optional docker login
# ---------------------------------------------------------------------------
if [ -n "${DOCKER_USERNAME:-}" ] && [ -n "${DOCKER_PASSWORD:-}" ]; then
    log "Authenticating to ${REGISTRY}..."
    run docker login "$REGISTRY" \
        --username "$DOCKER_USERNAME" \
        --password-stdin <<< "$DOCKER_PASSWORD"
fi

# ---------------------------------------------------------------------------
# Git SHA (must match build_images.sh)
# ---------------------------------------------------------------------------
cd "$REPO_ROOT"
GIT_SHA="$(git rev-parse --short=7 HEAD)"
if ! git diff --quiet HEAD 2>/dev/null; then
    GIT_SHA="${GIT_SHA}-dirty"
fi

# ---------------------------------------------------------------------------
# Service definitions (must stay in sync with build_images.sh)
# ---------------------------------------------------------------------------
declare -a SERVICES=(
    "control-plane|cargo|platform/control-plane/Cargo.toml"
    "identity-auth|cargo|platform/identity-auth/Cargo.toml"
    "ttp|cargo|modules/ttp/Cargo.toml"
    "ar|cargo|modules/ar/Cargo.toml"
    "payments|cargo|modules/payments/Cargo.toml"
    "tenant-control-plane-ui|npm|apps/tenant-control-plane-ui/package.json"
)

should_push() {
    local name="$1"
    if [ "${#TARGETS[@]}" -eq 0 ]; then return 0; fi
    for t in "${TARGETS[@]}"; do
        if [ "$t" = "$name" ]; then return 0; fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# Pre-flight: validate all tags before pushing anything
# ---------------------------------------------------------------------------
declare -a PUSH_LIST=()
PREFLIGHT_FAIL=false

for svc_def in "${SERVICES[@]}"; do
    IFS='|' read -r name vtype vfile <<< "$svc_def"

    if ! should_push "$name"; then continue; fi

    if [ ! -f "$REPO_ROOT/$vfile" ]; then
        log "ERROR: version file not found: $vfile" >&2
        PREFLIGHT_FAIL=true
        continue
    fi

    if [ "$vtype" = "cargo" ]; then
        version="$(extract_cargo_version "$REPO_ROOT/$vfile")"
    else
        version="$(extract_npm_version "$REPO_ROOT/$vfile")"
    fi

    tag="${version}-${GIT_SHA}"

    # Invariant 1: no 'latest'
    if [ "$tag" = "latest" ] || [[ "$tag" == *:latest ]]; then
        log "ERROR: Refusing to push 'latest' tag for ${name}." >&2
        PREFLIGHT_FAIL=true
        continue
    fi

    full_image="${REGISTRY}/${name}:${tag}"

    # Invariant 2: refuse to overwrite existing tag
    if ! $DRY_RUN && tag_exists_in_registry "$full_image"; then
        log "ERROR: Tag already exists in registry: ${full_image}" >&2
        log "       Refusing to overwrite. If this is intentional, delete the tag first." >&2
        PREFLIGHT_FAIL=true
        continue
    fi

    # Verify the image exists locally (was built by build_images.sh)
    if ! $DRY_RUN && ! docker image inspect "$full_image" >/dev/null 2>&1; then
        log "ERROR: Image not found locally: ${full_image}" >&2
        log "       Run build_images.sh first." >&2
        PREFLIGHT_FAIL=true
        continue
    fi

    PUSH_LIST+=("$full_image")
done

if $PREFLIGHT_FAIL; then
    log "Pre-flight checks failed. No images were pushed." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Preview
# ---------------------------------------------------------------------------
echo ""
echo "=== Images to push ==="
for img in "${PUSH_LIST[@]}"; do
    echo "  $img"
done
echo ""

if $DRY_RUN; then
    echo "(DRY-RUN — no images were pushed)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Push
# ---------------------------------------------------------------------------
PUSHED=()
for full_image in "${PUSH_LIST[@]}"; do
    log "Pushing: $full_image"
    run docker push "$full_image"
    PUSHED+=("$full_image")
    log "Pushed:  $full_image"
done

echo ""
echo "=== Push Summary ==="
echo "Pushed (${#PUSHED[@]}):"
for img in "${PUSHED[@]}"; do
    echo "  $img"
done
