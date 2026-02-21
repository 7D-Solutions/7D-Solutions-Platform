#!/usr/bin/env bash
# manifest_validate.sh — Verify all image tags in deploy/production/MODULE-MANIFEST.md exist in the registry.
#
# Parses the production manifest, extracts full image tags, and asserts each one
# exists via docker manifest inspect.
#
# Pending entries (SHA = "—" or tag contains "{sha}") emit warnings but do not
# fail. Once real tags are pinned, all must resolve.
#
# Usage:
#   bash scripts/production/manifest_validate.sh
#   bash scripts/production/manifest_validate.sh deploy/production/MODULE-MANIFEST.md
#   bash scripts/production/manifest_validate.sh --strict   # also fail on pending entries
#
# Environment:
#   IMAGE_REGISTRY   Override registry prefix (default: 7dsolutions)
#   MANIFEST_FILE    Override manifest path (default: deploy/production/MODULE-MANIFEST.md)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_FILE="${MANIFEST_FILE:-${REPO_ROOT}/deploy/production/MODULE-MANIFEST.md}"
STRICT=false

for arg in "$@"; do
    case "$arg" in
        --strict) STRICT=true ;;
        # Accept a file path as positional arg
        --*) echo "Unknown argument: $arg" >&2; exit 1 ;;
        *)   MANIFEST_FILE="$arg" ;;
    esac
done

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
    exit 1
fi

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[manifest_validate:prod] $*"; }
warn()   { echo "[manifest_validate:prod] WARN: $*" >&2; }

# ---------------------------------------------------------------------------
# Parse the manifest table
# Columns: | Description | `canonical` | version | sha | `full-image-tag` | notes |
# Output: "canonical_name|full_image_tag|sha_field"
# ---------------------------------------------------------------------------
parse_manifest() {
    grep '^|' "$MANIFEST_FILE" \
        | grep -v '^| Service\|^|---\|^| ---' \
        | awk -F'|' '{
            canonical = $3
            sha_field = $5
            full_tag  = $6
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", canonical)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", sha_field)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", full_tag)
            gsub(/^`|`$/, "", canonical)
            gsub(/^`|`$/, "", full_tag)
            if (length(canonical) > 0 && length(full_tag) > 0) {
                print canonical "|" full_tag "|" sha_field
            }
        }'
}

banner "Production Manifest: ${MANIFEST_FILE}"

PASS=0
FAIL=0
PENDING=0

while IFS='|' read -r canonical full_tag sha_field; do
    [[ -z "$canonical" ]] && continue

    # Detect pending entries: SHA column is "—" or tag contains "{sha}"
    is_pending=false
    if [[ "$sha_field" == "—" || "$full_tag" == *"{sha}"* ]]; then
        is_pending=true
    fi

    if $is_pending; then
        if $STRICT; then
            echo "  FAIL (pending) ${canonical} — tag not yet resolved: ${full_tag}"
            FAIL=$((FAIL + 1))
        else
            warn "Skipping pending entry: ${canonical} (${full_tag})"
            PENDING=$((PENDING + 1))
        fi
        continue
    fi

    # Check image exists in registry
    if docker manifest inspect "${full_tag}" > /dev/null 2>&1; then
        echo "  OK   ${canonical} → ${full_tag}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL ${canonical} → ${full_tag}  (image not found in registry)"
        FAIL=$((FAIL + 1))
    fi
done < <(parse_manifest)

echo ""
log "Results: ${PASS} OK / ${FAIL} FAIL / ${PENDING} PENDING"

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "ERROR: ${FAIL} image(s) are missing from the registry." >&2
    echo "       Images must be built and pushed to the registry before production deploy." >&2
    echo "       Build/push pipeline: .github/workflows/promote.yml" >&2
    exit 1
fi

if [[ $PASS -eq 0 && $PENDING -gt 0 && ! $STRICT ]]; then
    warn "All entries are pending (no real tags pinned yet). Nothing to validate."
    warn "Complete staging promotion and update the production manifest before deploying."
fi

log "Validation passed."
