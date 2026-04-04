#!/usr/bin/env bash
# check-port-catalog.sh — Validate port defaults against PLATFORM-SERVICE-CATALOG.md
#
# Parses the service catalog for canonical module→port mapping, then checks:
#   1. module.toml default_url ports match the catalog
#   2. PORT unwrap_or_else defaults in main.rs match the catalog
#   3. _BASE_URL unwrap_or_else defaults in Rust source match the catalog
#
# Usage:
#   ./scripts/check-port-catalog.sh          # run validation
#   ./scripts/check-port-catalog.sh --json   # machine-readable output
#
# Exit code: 0 if all ports match, 1 if any mismatch found.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CATALOG="$PROJECT_ROOT/docs/PLATFORM-SERVICE-CATALOG.md"

OUTPUT_JSON=false
[[ "${1:-}" == "--json" ]] && OUTPUT_JSON=true

if [ ! -f "$CATALOG" ]; then
  echo "ERROR: Service catalog not found at $CATALOG" >&2
  exit 2
fi

# ── Parse catalog ───────────────────────────────────────────────
# Build associative array: module_name → port
declare -A CATALOG_PORTS

while IFS='|' read -r _ module _ _ port _; do
  module=$(echo "$module" | xargs)   # trim whitespace
  port=$(echo "$port" | xargs)
  # Skip header/separator rows, empty ports, non-numeric ports
  [[ -z "$module" || "$module" == "Module" || "$module" == "-"* ]] && continue
  [[ -z "$port" || ! "$port" =~ ^[0-9]+$ ]] && continue
  CATALOG_PORTS["$module"]="$port"
done < <(grep -E '^\|' "$CATALOG" | grep -E '\|\s*[0-9]{4}\s*\|')

if [ ${#CATALOG_PORTS[@]} -eq 0 ]; then
  echo "ERROR: Could not parse any service→port entries from catalog" >&2
  exit 2
fi

# ── Helpers ─────────────────────────────────────────────────────
ERRORS=()
CHECKS=0

report_error() {
  local file="$1" detail="$2"
  ERRORS+=("$file: $detail")
}

# Resolve a dependency name to a catalog module name.
# e.g. "platform-client-tenant-registry" → "tenant-registry"
#      "ar" → "ar"
resolve_dep_name() {
  local dep="$1"
  # Direct match
  if [[ -n "${CATALOG_PORTS[$dep]+x}" ]]; then
    echo "$dep"
    return
  fi
  # Strip platform-client- prefix
  local stripped="${dep#platform-client-}"
  if [[ "$stripped" != "$dep" && -n "${CATALOG_PORTS[$stripped]+x}" ]]; then
    echo "$stripped"
    return
  fi
  # Not found in catalog (might be a library)
  echo ""
}

# Extract port from a URL like "http://localhost:8092" or "http://7d-party:8098"
extract_port() {
  echo "$1" | grep -oE ':[0-9]+' | tail -1 | tr -d ':'
}

# ── Check 1: module.toml default_url entries ────────────────────
check_module_toml() {
  local toml_files
  toml_files=$(find "$PROJECT_ROOT/modules" "$PROJECT_ROOT/platform" -name "module.toml" 2>/dev/null || true)

  while IFS= read -r toml; do
    [ -z "$toml" ] && continue
    # Parse lines with default_url
    while IFS= read -r line; do
      [ -z "$line" ] && continue
      # Extract dep name and URL from: dep_name = { ..., default_url = "http://..." }
      local dep_name url
      dep_name=$(echo "$line" | grep -oE '^[a-z][a-z0-9_-]*' | head -1)
      url=$(echo "$line" | grep -oE 'default_url\s*=\s*"[^"]+"' | sed 's/default_url\s*=\s*"//;s/"$//')
      [ -z "$dep_name" ] || [ -z "$url" ] && continue

      local port
      port=$(extract_port "$url")
      [ -z "$port" ] && continue

      local catalog_module
      catalog_module=$(resolve_dep_name "$dep_name")
      if [ -z "$catalog_module" ]; then
        # Dependency not in catalog (library, not a service) — skip
        continue
      fi

      local expected="${CATALOG_PORTS[$catalog_module]}"
      CHECKS=$((CHECKS + 1))
      if [ "$port" != "$expected" ]; then
        local relpath="${toml#$PROJECT_ROOT/}"
        report_error "$relpath" "dep '$dep_name' → port $port, catalog says $catalog_module=$expected"
      fi
    done < <(grep "default_url" "$toml" 2>/dev/null || true)
  done <<< "$toml_files"
}

# ── Check 2: PORT default in main.rs (own service port) ─────────
check_port_defaults() {
  # For each module/platform service with a main.rs, check if PORT default matches catalog
  local dirs=("$PROJECT_ROOT/modules" "$PROJECT_ROOT/platform")
  for base in "${dirs[@]}"; do
    [ -d "$base" ] || continue
    for main_rs in "$base"/*/src/main.rs; do
      [ -f "$main_rs" ] || continue
      local mod_dir
      mod_dir=$(basename "$(dirname "$(dirname "$main_rs")")")

      # Extract PORT default from: unwrap_or_else(|_| "NNNN")
      local port_default
      port_default=$(grep -E '"PORT".*unwrap_or_else' "$main_rs" 2>/dev/null \
        | grep -oE '"[0-9]{4}"' | tr -d '"' | head -1 || true)
      [ -z "$port_default" ] && continue

      if [[ -n "${CATALOG_PORTS[$mod_dir]+x}" ]]; then
        local expected="${CATALOG_PORTS[$mod_dir]}"
        CHECKS=$((CHECKS + 1))
        if [ "$port_default" != "$expected" ]; then
          local relpath="${main_rs#$PROJECT_ROOT/}"
          report_error "$relpath" "PORT default $port_default, catalog says $mod_dir=$expected"
        fi
      fi
    done
  done
}

# ── Check 3: _BASE_URL defaults in source code ──────────────────
check_base_url_defaults() {
  # Find patterns like: env::var("FOO_BASE_URL").unwrap_or_else(|_| "http://localhost:NNNN")
  # Map FOO to a catalog module name via heuristic (FOO_BASE_URL → foo → module)
  local dirs=("$PROJECT_ROOT/modules" "$PROJECT_ROOT/platform")
  for base in "${dirs[@]}"; do
    [ -d "$base" ] || continue
    while IFS= read -r match; do
      [ -z "$match" ] && continue
      local file line_content
      file=$(echo "$match" | cut -d: -f1)
      line_content=$(echo "$match" | cut -d: -f2-)

      # Extract env var name: "FOO_BASE_URL"
      local env_var
      env_var=$(echo "$line_content" | grep -oE '"[A-Z_]+_BASE_URL"' | tr -d '"' | head -1)
      [ -z "$env_var" ] && continue

      # Extract port from the default URL
      local port
      port=$(echo "$line_content" | grep -oE 'localhost:[0-9]{4}' | grep -oE '[0-9]{4}' | head -1)
      [ -z "$port" ] && continue

      # Derive module name: strip _BASE_URL, lowercase, replace _ with -
      local mod_name
      mod_name=$(echo "${env_var%_BASE_URL}" | tr '[:upper:]' '[:lower:]' | tr '_' '-')

      if [[ -n "${CATALOG_PORTS[$mod_name]+x}" ]]; then
        local expected="${CATALOG_PORTS[$mod_name]}"
        CHECKS=$((CHECKS + 1))
        if [ "$port" != "$expected" ]; then
          local relpath="${file#$PROJECT_ROOT/}"
          report_error "$relpath" "$env_var default localhost:$port, catalog says $mod_name=$expected"
        fi
      fi
    done < <(grep -rn '_BASE_URL.*unwrap_or_else.*localhost:[0-9]\{4\}' "$base" 2>/dev/null \
             | grep -v '/target/' || true)
  done
}

# ── Run all checks ──────────────────────────────────────────────
check_module_toml
check_port_defaults
check_base_url_defaults

# ── Output ──────────────────────────────────────────────────────
if $OUTPUT_JSON; then
  echo "{"
  echo "  \"timestamp\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
  echo "  \"catalog_entries\": ${#CATALOG_PORTS[@]},"
  echo "  \"checks_run\": $CHECKS,"
  echo "  \"errors\": ${#ERRORS[@]},"
  echo "  \"details\": ["
  for i in "${!ERRORS[@]}"; do
    comma=""
    [ "$i" -lt $((${#ERRORS[@]} - 1)) ] && comma=","
    echo "    \"${ERRORS[$i]}\"$comma"
  done
  echo "  ]"
  echo "}"
else
  echo "# Port Catalog Validation"
  echo ""
  echo "**Catalog entries:** ${#CATALOG_PORTS[@]} | **Checks:** $CHECKS | **Errors:** ${#ERRORS[@]}"
  echo ""
  if [ ${#ERRORS[@]} -eq 0 ]; then
    echo "All port defaults match the catalog."
  else
    echo "## Mismatches"
    echo ""
    for err in "${ERRORS[@]}"; do
      echo "- $err"
    done
  fi
fi

# Exit code
[ ${#ERRORS[@]} -eq 0 ] && exit 0 || exit 1
