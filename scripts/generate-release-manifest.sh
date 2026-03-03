#!/usr/bin/env bash
set -euo pipefail

# Generate a deterministic release manifest JSON from the current commit.
# Same commit always produces the same manifest (no timestamps from system clock).
#
# Usage:
#   bash scripts/generate-release-manifest.sh [--release-name NAME] [--out FILE]
#
# Defaults:
#   --release-name  fireproof-go-live
#   --out           docs/releases/<release-name>-manifest.json

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_NAME="fireproof-go-live"
OUT_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release-name) RELEASE_NAME="$2"; shift 2 ;;
    --out)          OUT_FILE="$2"; shift 2 ;;
    *)              echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$OUT_FILE" ]]; then
  OUT_FILE="$PROJECT_ROOT/docs/releases/${RELEASE_NAME}-manifest.json"
fi

mkdir -p "$(dirname "$OUT_FILE")"

COMMIT_SHA=$(git -C "$PROJECT_ROOT" rev-parse HEAD)
COMMIT_SHA7=$(git -C "$PROJECT_ROOT" rev-parse --short=7 HEAD)
COMMIT_DATE=$(git -C "$PROJECT_ROOT" log -1 --format=%cI HEAD)
REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

# Collect crate entries as tab-separated lines: name\tcategory\tversion\tproven
collect_lines() {
  local base_dir="$1"
  local category="$2"
  for toml in "$PROJECT_ROOT/$base_dir"/*/Cargo.toml; do
    [[ -f "$toml" ]] || continue
    local name
    name=$(basename "$(dirname "$toml")")
    local version
    version=$(grep '^version' "$toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
    local proven="false"
    # Check if version >= 1.0.0
    local major="${version%%.*}"
    if [[ "$major" -ge 1 ]] 2>/dev/null; then
      proven="true"
    fi
    printf '%s\t%s\t%s\t%s\n' "$name" "$category" "$version" "$proven"
  done
}

# Collect all lines, sort by name for determinism
ALL_LINES=$(
  {
    collect_lines "modules" "module"
    collect_lines "platform" "platform"
  } | sort -t$'\t' -k1,1
)

TOTAL=$(echo "$ALL_LINES" | wc -l | tr -d ' ')
COUNT=0

{
  cat <<HEADER
{
  "release_name": "${RELEASE_NAME}",
  "release_version": "1.0.0",
  "commit_sha": "${COMMIT_SHA}",
  "commit_sha7": "${COMMIT_SHA7}",
  "commit_date": "${COMMIT_DATE}",
  "registry": "${REGISTRY}",
  "tag_name": "${RELEASE_NAME}-v1.0.0",
  "rollback_tag": null,
  "components": [
HEADER

  while IFS=$'\t' read -r name category version proven; do
    COUNT=$((COUNT + 1))
    image_tag="${REGISTRY}/${name}:${version}-${COMMIT_SHA7}"
    comma=","
    if [[ "$COUNT" -eq "$TOTAL" ]]; then
      comma=""
    fi
    cat <<ENTRY
    {
      "name": "${name}",
      "category": "${category}",
      "version": "${version}",
      "proven": ${proven},
      "image_tag": "${image_tag}"
    }${comma}
ENTRY
  done <<< "$ALL_LINES"

  echo "  ]"
  echo "}"
} > "$OUT_FILE"

echo "Manifest written to: $OUT_FILE"
echo "Commit: $COMMIT_SHA"
echo "Components: $TOTAL"
