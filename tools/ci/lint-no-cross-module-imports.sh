#!/usr/bin/env bash
#
# Lint: Forbid cross-module imports
#
# Phase 16: Enforce module boundaries to prevent coupling.
# Modules communicate via events and contracts, NOT source imports.
#
# Forbidden: AR importing from Payments source, etc.
# Allowed:   Shared libraries (event_bus, sqlx), contracts, same-module imports
#
# Exit 0: Clean (no violations)
# Exit 1: Violations found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "🔍 Linting: Checking for cross-module imports..."

VIOLATIONS=""

# Module crate names
MODULES=("ar_rs" "payments_rs" "subscriptions_rs" "notifications_rs" "gl_rs")
MODULE_DIRS=("ar" "payments" "subscriptions" "notifications" "gl")

# For each module, check if it imports from other modules
for i in "${!MODULES[@]}"; do
    current_module="${MODULES[$i]}"
    current_dir="${MODULE_DIRS[$i]}"
    
    # Build list of OTHER module crates to check for
    other_modules=()
    for j in "${!MODULES[@]}"; do
        if [ "$i" != "$j" ]; then
            other_modules+=("${MODULES[$j]}")
        fi
    done
    
    # Search for imports from other modules in current module's source
    for other in "${other_modules[@]}"; do
        # Look for "use other_module::" in source files
        # Exclude comments (lines starting with //)
        matches=$(grep -r "^[[:space:]]*use ${other}::" \
            "modules/${current_dir}/src" \
            --include="*.rs" \
            2>/dev/null \
            | grep -v "^[[:space:]]*//" \
            || true)
        
        if [ -n "$matches" ]; then
            VIOLATIONS+="❌ Module '${current_dir}' imports from '${other}':\n${matches}\n\n"
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

echo "✅ Lint passed: No cross-module imports detected"
exit 0
