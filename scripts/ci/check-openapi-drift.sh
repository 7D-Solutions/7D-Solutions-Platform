#!/usr/bin/env bash
# CI gate: detect drift between checked-in clients/*/openapi.json and the
# actual OpenAPI spec produced by each module's openapi_dump binary.
#
# Usage:
#   scripts/ci/check-openapi-drift.sh              # check default modules
#   OPENAPI_MODULES="ar ap gl" scripts/ci/...       # override module list
set -euo pipefail

# High-churn HTTP surfaces checked by default.  Extend via env var.
MODULES=(${OPENAPI_MODULES:-ar ap payments})

errors=()

for mod in "${MODULES[@]}"; do
  cargo_toml="modules/$mod/Cargo.toml"
  client_spec="clients/$mod/openapi.json"
  dump_src="modules/$mod/src/bin/openapi_dump.rs"

  # --- pre-flight checks ---------------------------------------------------
  if [ ! -f "$cargo_toml" ]; then
    errors+=("$mod: missing $cargo_toml")
    continue
  fi
  if [ ! -f "$dump_src" ]; then
    errors+=("$mod: missing openapi_dump binary at $dump_src")
    continue
  fi
  if [ ! -f "$client_spec" ]; then
    errors+=("$mod: missing checked-in spec at $client_spec")
    continue
  fi

  # Extract crate package name from Cargo.toml (portable across GNU/BSD)
  pkg=$(awk '/^\[package\]/{found=1} found && /^name/{split($0,a,"\""); print a[2]; exit}' "$cargo_toml")
  if [ -z "$pkg" ]; then
    errors+=("$mod: could not extract package name from $cargo_toml")
    continue
  fi

  echo "--- $mod (package: $pkg) ---"

  # Build + run the dump binary
  actual=$(cargo run -p "$pkg" --bin openapi_dump 2>/dev/null) || {
    errors+=("$mod: cargo run -p $pkg --bin openapi_dump failed")
    continue
  }

  # Normalise both specs through jq so key order doesn't matter
  expected=$(jq --sort-keys . "$client_spec")
  actual_sorted=$(echo "$actual" | jq --sort-keys .)

  if [ "$expected" != "$actual_sorted" ]; then
    echo "DRIFT DETECTED in $mod"
    diff <(echo "$expected") <(echo "$actual_sorted") || true
    errors+=("$mod: checked-in $client_spec differs from openapi_dump output — run 'cargo run -p $pkg --bin openapi_dump | jq --sort-keys . > $client_spec' to fix")
  else
    echo "$mod: OK"
  fi
done

echo ""
if [ ${#errors[@]} -gt 0 ]; then
  echo "FAILED — OpenAPI drift detected:" >&2
  printf '  %s\n' "${errors[@]}" >&2
  exit 1
fi

echo "All checked modules match their checked-in OpenAPI specs."
