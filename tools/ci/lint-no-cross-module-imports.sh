#!/usr/bin/env bash
#
# Lint: Forbid cross-module imports
#
# Enforce module boundaries to prevent coupling.
# Modules communicate via events and contracts, NOT source imports.
#
# Forbidden: AR importing from Payments source, etc.
# Allowed:   Shared libraries (event_bus, sqlx), contracts, same-module imports
#
# Modules are auto-discovered from modules/ directory.
#
# Exit 0: Clean (no violations)
# Exit 1: Violations found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "🔍 Linting: Checking for cross-module imports..."

VIOLATIONS=""

# Auto-discover modules and their crate names from Cargo.toml
declare -a MODULE_DIRS=()
declare -a MODULE_CRATES=()

for cargo_toml in modules/*/Cargo.toml; do
    dir_name=$(basename "$(dirname "$cargo_toml")")
    # Extract crate name: line matching ^name = "..."
    crate_name=$(grep -m1 '^name\s*=' "$cargo_toml" | sed 's/^name\s*=\s*"\(.*\)"/\1/' | tr '-' '_')
    if [ -n "$crate_name" ] && [ -d "modules/${dir_name}/src" ]; then
        MODULE_DIRS+=("$dir_name")
        MODULE_CRATES+=("$crate_name")
    fi
done

# Known allowlist: GL has a legitimate dev-dependency on AP for test harness
# Format: "importing_crate:imported_crate" (underscore names)
ALLOWLIST=(
    "gl_rs:ap"  # GL's e2e_ap_bill_payment_gl.rs test harness
)

is_allowed() {
    local importer="$1"
    local imported="$2"
    for entry in "${ALLOWLIST[@]}"; do
        if [ "$entry" = "${importer}:${imported}" ]; then
            return 0
        fi
    done
    return 1
}

# For each module, check if it imports from other modules
for i in "${!MODULE_CRATES[@]}"; do
    current_crate="${MODULE_CRATES[$i]}"
    current_dir="${MODULE_DIRS[$i]}"

    for j in "${!MODULE_CRATES[@]}"; do
        if [ "$i" = "$j" ]; then
            continue
        fi
        other_crate="${MODULE_CRATES[$j]}"

        # Skip allowlisted pairs
        if is_allowed "$current_crate" "$other_crate"; then
            continue
        fi

        # Look for "use other_module::" in source files (exclude comments)
        matches=$(grep -r "^[[:space:]]*use ${other_crate}::" \
            "modules/${current_dir}/src" \
            --include="*.rs" \
            2>/dev/null \
            | grep -v "^[[:space:]]*//" \
            || true)

        if [ -n "$matches" ]; then
            VIOLATIONS+="❌ Module '${current_dir}' imports from '${other_crate}':\n${matches}\n\n"
        fi
    done
done

if [ -n "$VIOLATIONS" ]; then
    echo "❌ LINT FAILURE: Cross-module imports detected"
    echo ""
    echo -e "$VIOLATIONS"
    echo "Modules must communicate via:"
    echo "  - Event bus (event_bus crate)"
    echo "  - Contracts (shared interfaces)"
    echo "  - HTTP APIs (cross-service boundaries)"
    echo ""
    echo "Forbidden: Direct source imports between modules"
    echo "Rationale: Preserves module boundaries, enables independent deployment"
    exit 1
fi

echo "✅ Lint passed: No cross-module imports detected (${#MODULE_CRATES[@]} modules checked)"
exit 0
