#!/usr/bin/env bash
# Gate 2 extension: publish platform crates to the 7d-platform registry.
#
# Publishes all platform/* crates that declare `publish = ["7d-platform"]`.
# Order matters — crates are published in dependency order so that
# dependents find their deps already in the registry.
#
# Requires: CARGO_REGISTRIES_7D_PLATFORM_TOKEN env var (GitHub PAT with
# write:packages scope).
#
# Immutability: `cargo publish` will fail (exit 0 skipped) if a version
# already exists in the registry. This is the desired behaviour — we never
# overwrite a published version.
set -euo pipefail

if [[ -z "${CARGO_REGISTRIES_7D_PLATFORM_TOKEN:-}" ]]; then
  echo "ERROR: CARGO_REGISTRIES_7D_PLATFORM_TOKEN is not set" >&2
  exit 1
fi

# Dependency order: leaf crates first, then crates that depend on them.
CRATES=(
  platform/health
  platform/tax-core
  platform/event-bus
  platform/platform-contracts
  platform/audit
  platform/projections
  platform/security
  platform/tenant-registry
  platform/control-plane
  platform/identity-auth
  platform/doc-mgmt
)

published=0
skipped=0
failed=0

for crate_dir in "${CRATES[@]}"; do
  toml="$crate_dir/Cargo.toml"
  if [[ ! -f "$toml" ]]; then
    echo "SKIP $crate_dir — no Cargo.toml"
    skipped=$((skipped + 1))
    continue
  fi

  # Only publish crates that opt in
  if ! grep -q 'publish.*=.*\["7d-platform"\]' "$toml"; then
    echo "SKIP $crate_dir — not marked for publishing"
    skipped=$((skipped + 1))
    continue
  fi

  name=$(grep '^name' "$toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
  version=$(grep '^version' "$toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')

  echo "--- Publishing $name@$version from $crate_dir ---"

  # cargo publish returns non-zero if the version already exists.
  # We treat "already exists" as a skip, not a failure.
  if cargo publish --registry 7d-platform --manifest-path "$toml" --allow-dirty 2>&1 | tee /tmp/publish-output.txt; then
    echo "OK $name@$version published"
    published=$((published + 1))
  else
    if grep -qi "already exists\|already uploaded" /tmp/publish-output.txt; then
      echo "SKIP $name@$version — already published (immutable)"
      skipped=$((skipped + 1))
    else
      echo "FAIL $name@$version" >&2
      failed=$((failed + 1))
    fi
  fi
done

echo ""
echo "=== Publish summary: $published published, $skipped skipped, $failed failed ==="

if [[ $failed -gt 0 ]]; then
  exit 1
fi
