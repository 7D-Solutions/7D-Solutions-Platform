#!/usr/bin/env bash
#
# Lint: Warn on unstructured tracing::error! / tracing::warn! calls in HTTP handlers.
#
# The platform logging standard (docs/architecture/LOGGING-STANDARD.md) requires
# that ERROR calls include an error_code field and that both ERROR and WARN calls
# avoid bare string-only messages.
#
# What this script flags (bare string-only messages):
#   tracing::error!("some message")
#   tracing::warn!("some message")
#
# What it does NOT flag (calls with at least one field are fine):
#   tracing::error!(error_code = "X", "msg")
#   tracing::warn!(count = n, "cache miss")
#   tracing::error!(error = %e, "database error")
#
# Scope: modules/*/src/http/**/*.rs (handler files only)
# Background tasks and consumers are out of scope; they use ctx.log_span().
#
# Exit 0: Clean
# Exit 1: Violations found

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "🔍 Checking structured log fields in HTTP handler files..."

# Pattern: tracing::{error,warn}! followed by open paren, optional whitespace,
# then a string literal (no field = value pairs before the message).
# This catches:  tracing::error!("msg")  tracing::warn!("msg {}", x)
# but not:       tracing::error!(field = v, "msg")
BARE_PATTERN='tracing::(error|warn)!\s*\(\s*"'

VIOLATIONS=$(grep -rn --include="*.rs" -E "$BARE_PATTERN" \
    modules/*/src/http/ \
    2>/dev/null || true)

if [ -n "$VIOLATIONS" ]; then
    echo ""
    echo "❌ LINT FAILURE: Unstructured log calls found in HTTP handler files"
    echo ""
    echo "Violations (add structured fields before the message string):"
    echo "$VIOLATIONS"
    echo ""
    echo "Fix examples:"
    echo "  tracing::error!(error_code = \"ALLOC_FAILED\", error = %e, \"allocation failed\");"
    echo "  tracing::warn!(tenant_id = %tenant_id, \"rate limit approaching\");"
    echo ""
    echo "See docs/architecture/LOGGING-STANDARD.md for required fields per level."
    exit 1
fi

echo "✅ Lint passed: All HTTP handler log calls include structured fields"
exit 0
