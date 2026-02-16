#!/usr/bin/env bash
#
# Lint: Forbid raw PgPool creation outside db/resolver.rs or db.rs
#
# Phase 16: Enforce resolver pattern for all DB pool creation.
# This lint ensures modules use the centralized resolver, enabling
# future isolation tiers (PDAA) without code changes.
#
# Allowed:    modules/*/src/db/resolver.rs (standard pattern)
#             modules/*/src/db.rs (legacy pattern, GL only)
# Forbidden:  All other locations (main.rs, lib.rs, services/*, bin/*, etc.)
#
# Exit 0: Clean (no violations)
# Exit 1: Violations found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "🔍 Linting: Checking for raw PgPool creation outside db/resolver.rs or db.rs..."

# Find all Rust files containing PgPoolOptions::new() 
# Exclude:
# - */db/resolver.rs (allowed - standard pattern)
# - */db.rs (allowed - legacy pattern)
# - *.bak, *.orig (backup files)
# - test files in tests/ directories (tests can create pools directly)
# - e2e-tests (integration tests)

VIOLATIONS=$(grep -r "PgPoolOptions::new()" \
    --include="*.rs" \
    modules/ \
    | grep -v "/bin/" \
    | grep -v "/db/resolver.rs:" \
    | grep -v "/db.rs:" \
    | grep -v "\.bak" \
    | grep -v "\.orig" \
    | grep -v "/tests/" \
    | grep -v "^e2e-tests/" \
    || true)

if [ -n "$VIOLATIONS" ]; then
    echo "❌ LINT FAILURE: Raw PgPool creation found outside db/resolver.rs or db.rs"
    echo ""
    echo "Violations:"
    echo "$VIOLATIONS"
    echo ""
    echo "Fix: Use the centralized resolver pattern instead:"
    echo "  use crate::db::resolver::create_pool;"
    echo "  let pool = create_pool().await?;"
    echo ""
    echo "Or for modules with legacy db.rs:"
    echo "  use crate::db::create_pool;"
    echo "  let pool = create_pool().await?;"
    echo ""
    echo "Rationale: Centralized pool creation enables future isolation tiers (PDAA)"
    echo "without changing calling code. See docs/governance/DOMAIN-OWNERSHIP.md"
    exit 1
fi

echo "✅ Lint passed: All modules use resolver pattern"
exit 0
