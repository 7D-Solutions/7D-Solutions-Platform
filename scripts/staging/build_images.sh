#!/usr/bin/env bash
# build_images.sh — Build immutable Docker images for all staging services.
#
# Tags each image with: {version}-{git-sha7}  (never 'latest')
# Rust services build from the workspace root using workspace Dockerfiles.
# identity-auth and payments build from their own directories (standalone).
# TCP UI builds from apps/tenant-control-plane-ui/.
#
# Rust compilation inside Docker is isolated — each container has its own
# cargo registry and target dir. The ./scripts/cargo-slot.sh system governs
# local (non-Docker) cargo invocations; it is not needed inside Docker builds.
#
# Usage:
#   bash scripts/staging/build_images.sh              # build all
#   bash scripts/staging/build_images.sh --dry-run    # print commands, do not execute
#   bash scripts/staging/build_images.sh control-plane ar  # build specific services
#
# Environment:
#   IMAGE_REGISTRY  Registry prefix (default: 7dsolutions)
#   DOCKER_BUILD_ARGS  Extra args passed to docker build (e.g., --no-cache)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DRY_RUN=false
TARGETS=()

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
        --*) echo "Unknown flag: $arg" >&2; exit 1 ;;
        *) TARGETS+=("$arg") ;;
    esac
done

REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"
EXTRA_BUILD_ARGS="${DOCKER_BUILD_ARGS:-}"

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

log() { echo "[build_images] $*"; }
run() {
    if $DRY_RUN; then
        echo "  [DRY-RUN] $*"
    else
        "$@"
    fi
}

# ---------------------------------------------------------------------------
# Git SHA
# ---------------------------------------------------------------------------
cd "$REPO_ROOT"
GIT_SHA="$(git rev-parse --short=7 HEAD)"
if ! git diff --quiet HEAD 2>/dev/null; then
    GIT_SHA="${GIT_SHA}-dirty"
    log "WARN: working tree is dirty — tag will include '-dirty' suffix"
fi

# ---------------------------------------------------------------------------
# Service build specifications
#
# Fields: canonical_name | vtype | vfile | dockerfile | build_context
#   vtype: cargo | npm
#   dockerfile: path relative to REPO_ROOT
#   build_context: path relative to REPO_ROOT ('.' = workspace root)
# ---------------------------------------------------------------------------
declare -a SERVICES=(
    "control-plane|cargo|platform/control-plane/Cargo.toml|platform/control-plane/Dockerfile.workspace|."
    "identity-auth|cargo|platform/identity-auth/Cargo.toml|platform/identity-auth/deploy/Dockerfile|platform/identity-auth"
    "ttp|cargo|modules/ttp/Cargo.toml|modules/ttp/Dockerfile.workspace|."
    "ar|cargo|modules/ar/Cargo.toml|modules/ar/deploy/Dockerfile.workspace|."
    "payments|cargo|modules/payments/Cargo.toml|modules/payments/Dockerfile|modules/payments"
    "tenant-control-plane-ui|npm|apps/tenant-control-plane-ui/package.json|apps/tenant-control-plane-ui/Dockerfile|apps/tenant-control-plane-ui"
)

# ---------------------------------------------------------------------------
# Filter to requested targets (if any)
# ---------------------------------------------------------------------------
should_build() {
    local name="$1"
    if [ "${#TARGETS[@]}" -eq 0 ]; then return 0; fi
    for t in "${TARGETS[@]}"; do
        if [ "$t" = "$name" ]; then return 0; fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# Build loop
# ---------------------------------------------------------------------------
BUILT=()
SKIPPED=()

for svc_def in "${SERVICES[@]}"; do
    IFS='|' read -r name vtype vfile dockerfile build_ctx <<< "$svc_def"

    if ! should_build "$name"; then
        SKIPPED+=("$name")
        continue
    fi

    if [ ! -f "$REPO_ROOT/$vfile" ]; then
        log "ERROR: version file not found: $vfile" >&2
        exit 1
    fi
    if [ ! -f "$REPO_ROOT/$dockerfile" ]; then
        log "ERROR: Dockerfile not found: $dockerfile" >&2
        exit 1
    fi

    if [ "$vtype" = "cargo" ]; then
        version="$(extract_cargo_version "$REPO_ROOT/$vfile")"
    else
        version="$(extract_npm_version "$REPO_ROOT/$vfile")"
    fi

    tag="${version}-${GIT_SHA}"
    full_image="${REGISTRY}/${name}:${tag}"

    log "Building ${name} → ${full_image}"
    log "  Dockerfile:    $dockerfile"
    log "  Build context: $build_ctx"

    # shellcheck disable=SC2086
    run docker build \
        --file "$REPO_ROOT/$dockerfile" \
        --tag "$full_image" \
        --label "org.opencontainers.image.revision=$(git rev-parse HEAD)" \
        --label "org.opencontainers.image.version=${version}" \
        --label "org.opencontainers.image.source=https://github.com/7d-solutions/platform" \
        $EXTRA_BUILD_ARGS \
        "$REPO_ROOT/$build_ctx"

    log "Built: $full_image"
    BUILT+=("$full_image")
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Build Summary ==="
if $DRY_RUN; then echo "(DRY-RUN — no images were actually built)"; fi
echo "Built (${#BUILT[@]}):"
for img in "${BUILT[@]}"; do
    echo "  $img"
done
if [ "${#SKIPPED[@]}" -gt 0 ]; then
    echo "Skipped (${#SKIPPED[@]}): ${SKIPPED[*]}"
fi
