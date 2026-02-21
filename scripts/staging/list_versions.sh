#!/usr/bin/env bash
# list_versions.sh — Print resolved image names and tags for all staging artifacts.
#
# No build is performed. Outputs what build_images.sh would produce.
# Tags are immutable: {version}-{git-sha7}
#
# Usage: bash scripts/staging/list_versions.sh [--json]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUTPUT_JSON=false

for arg in "$@"; do
    case "$arg" in
        --json) OUTPUT_JSON=true ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

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

# ---------------------------------------------------------------------------
# Registry prefix (override via IMAGE_REGISTRY env var)
# ---------------------------------------------------------------------------
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

# ---------------------------------------------------------------------------
# Git SHA (7 chars)
# ---------------------------------------------------------------------------
cd "$REPO_ROOT"
GIT_SHA="$(git rev-parse --short=7 HEAD)"
if ! git diff --quiet HEAD 2>/dev/null; then
    GIT_SHA="${GIT_SHA}-dirty"
fi

# ---------------------------------------------------------------------------
# Service definitions
# Format: "canonical_name|version_file_type|version_file_path"
# ---------------------------------------------------------------------------
declare -a SERVICES=(
    "control-plane|cargo|platform/control-plane/Cargo.toml"
    "identity-auth|cargo|platform/identity-auth/Cargo.toml"
    "ttp|cargo|modules/ttp/Cargo.toml"
    "ar|cargo|modules/ar/Cargo.toml"
    "payments|cargo|modules/payments/Cargo.toml"
    "tenant-control-plane-ui|npm|apps/tenant-control-plane-ui/package.json"
)

# ---------------------------------------------------------------------------
# Resolve and print
# ---------------------------------------------------------------------------
if $OUTPUT_JSON; then
    echo "{"
    echo "  \"git_sha\": \"${GIT_SHA}\","
    echo "  \"registry\": \"${REGISTRY}\","
    echo "  \"images\": ["
    count=0
    total=${#SERVICES[@]}
fi

for svc_def in "${SERVICES[@]}"; do
    IFS='|' read -r name vtype vfile <<< "$svc_def"

    if [ ! -f "$REPO_ROOT/$vfile" ]; then
        echo "WARN: version file not found: $vfile" >&2
        continue
    fi

    if [ "$vtype" = "cargo" ]; then
        version="$(extract_cargo_version "$REPO_ROOT/$vfile")"
    else
        version="$(extract_npm_version "$REPO_ROOT/$vfile")"
    fi

    tag="${version}-${GIT_SHA}"
    full_image="${REGISTRY}/${name}:${tag}"

    if $OUTPUT_JSON; then
        count=$((count + 1))
        comma=""
        if [ "$count" -lt "$total" ]; then comma=","; fi
        printf '    {"name": "%s", "version": "%s", "tag": "%s", "image": "%s"}%s\n' \
            "$name" "$version" "$tag" "$full_image" "$comma"
    else
        printf "%-32s  %s\n" "$name" "$full_image"
    fi
done

if $OUTPUT_JSON; then
    echo "  ]"
    echo "}"
fi
