#!/usr/bin/env bash
# manifest_validate.sh — Verify all image tags in MODULE-MANIFEST.md exist in the registry.
#
# Parses deploy/staging/MODULE-MANIFEST.md, extracts full image tags,
# and asserts each one exists (via docker manifest inspect).
#
# Pending entries (SHA = "—" or tag contains "{sha}") emit warnings but
# do not fail. Once real tags are pinned, all must resolve.
#
# Usage:
#   bash scripts/staging/manifest_validate.sh
#   bash scripts/staging/manifest_validate.sh --strict   # also fail on pending entries
#
# Environment:
#   IMAGE_REGISTRY   Override registry prefix (default: 7dsolutions)
#   MANIFEST_FILE    Override manifest path (default: deploy/staging/MODULE-MANIFEST.md)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_FILE="${MANIFEST_FILE:-${REPO_ROOT}/deploy/staging/MODULE-MANIFEST.md}"
STRICT=false

for arg in "$@"; do
    case "$arg" in
        --strict) STRICT=true ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "ERROR: Manifest not found: ${MANIFEST_FILE}" >&2
    exit 1
fi

banner() { echo ""; echo "=== $* ==="; }
log()    { echo "[manifest_validate] $*"; }
warn()   { echo "[manifest_validate] WARN: $*" >&2; }

# ---------------------------------------------------------------------------
# Parse the manifest table
# Extract rows: | Description | `canonical-name` | version | sha | `full-image-tag` | notes |
# Skip header rows and separator rows.
# ---------------------------------------------------------------------------
# Output format per line: "canonical_name|full_image_tag|sha_field"
parse_manifest() {
    grep '^|' "$MANIFEST_FILE" \
        | grep -v '^| Service\|^|---\|^| ---' \
        | awk -F'|' '{
            # Columns (1-indexed after leading |):
            # $2 = description, $3 = canonical, $4 = version, $5 = sha, $6 = full_image_tag, $7 = notes
            canonical = $3
            sha_field = $5
            full_tag  = $6
            # Strip whitespace
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", canonical)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", sha_field)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", full_tag)
            # Strip backtick wrappers
            gsub(/^`|`$/, "", canonical)
            gsub(/^`|`$/, "", full_tag)
            if (length(canonical) > 0 && length(full_tag) > 0) {
                print canonical "|" full_tag "|" sha_field
            }
        }'
}

banner "Manifest: ${MANIFEST_FILE}"

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
    echo "       Push them with: bash scripts/staging/push_images.sh" >&2
    exit 1
fi

if [[ $PASS -eq 0 && $PENDING -gt 0 && ! $STRICT ]]; then
    warn "All entries are pending (no real tags pinned yet). Nothing to validate."
    warn "Run 'bash scripts/staging/push_images.sh' then update the manifest."
fi

log "Validation passed."
