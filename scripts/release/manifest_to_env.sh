#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-deploy/production/compose-release-manifest.json}"
OUT_PATH="${2:-.env.release}"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "manifest_to_env.sh: manifest not found: $MANIFEST_PATH" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "manifest_to_env.sh: jq is required" >&2
  exit 1
fi

jq -e '.services | type == "array" and length > 0' "$MANIFEST_PATH" >/dev/null

{
  echo "# Generated from $MANIFEST_PATH"
  echo "# generated_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  echo "RELEASE_TAG=$(jq -r '.release_tag' "$MANIFEST_PATH")"
  echo "RELEASE_GIT_SHA=$(jq -r '.git_sha' "$MANIFEST_PATH")"
  jq -r '.services[] | "\(.env_var)=\(.image)"' "$MANIFEST_PATH" | awk '!seen[$0]++'
} > "$OUT_PATH"

echo "Wrote $OUT_PATH from $MANIFEST_PATH"
