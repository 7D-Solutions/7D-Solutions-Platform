#!/usr/bin/env bash
# CI check: verify every Rust crate under clients/ is a workspace member.
# Prevents crates from silently falling out of the workspace build.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

missing=()

for cargo_file in "$REPO_ROOT"/clients/*/Cargo.toml; do
  # Extract the relative path (e.g. "clients/party")
  crate_dir="$(dirname "$cargo_file")"
  rel_path="${crate_dir#"$REPO_ROOT/"}"

  if ! grep -qF "\"$rel_path\"" "$CARGO_TOML"; then
    missing+=("$rel_path")
  fi
done

if [ ${#missing[@]} -gt 0 ]; then
  echo "ERROR: The following client crates are not workspace members:" >&2
  for m in "${missing[@]}"; do
    echo "  - $m" >&2
  done
  echo "" >&2
  echo "Add them to the [workspace] members list in Cargo.toml." >&2
  exit 1
fi

echo "✓ All client crates are workspace members"
