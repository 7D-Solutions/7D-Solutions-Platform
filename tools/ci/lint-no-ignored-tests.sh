#!/usr/bin/env bash
#
# Lint: Forbid new #[ignore] attributes in E2E tests
#
# Phase 19: Prevent test coverage regression by blocking unapproved #[ignore].
#
# Rationale: Ignored tests are invisible to the default cargo test sweep.
# Any test that needs to be skipped must be explicitly approved and added to
# the allowlist below. This prevents silent coverage gaps.
#
# Allowlist: tests that are legitimately ignored (e.g. require live infra)
# - real_e2e.rs: test_real_nats_based_e2e (requires live NATS cluster)
#
# Exit 0: Clean (no unapproved #[ignore] found)
# Exit 1: Unapproved #[ignore] detected

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "🔍 Linting: Checking for unapproved #[ignore] in e2e-tests..."

E2E_DIR="e2e-tests/tests"

# Allowlist: file:line patterns that are permitted to have #[ignore]
# Format: "filename.rs:line_content_pattern"
# Use grep -F for literal string matching
ALLOWLIST=(
    "real_e2e.rs"
)

VIOLATIONS=""

# Find all #[ignore] attribute occurrences in e2e-tests/tests/
# Exclude comment lines (// and ///) that mention #[ignore] in documentation
while IFS=: read -r filepath _; do
    filename="$(basename "$filepath")"

    # Check if this file is in the allowlist
    allowed=false
    for allowed_file in "${ALLOWLIST[@]}"; do
        if [ "$filename" = "$allowed_file" ]; then
            allowed=true
            break
        fi
    done

    if [ "$allowed" = false ]; then
        VIOLATIONS+="❌ Unapproved #[ignore] in: ${filepath}\n"
    fi
done < <(grep -rn '#\[ignore\]' "$E2E_DIR" --include="*.rs" 2>/dev/null \
    | grep -v '^\s*//' \
    | grep -v '///' \
    || true)

if [ -n "$VIOLATIONS" ]; then
    echo "❌ LINT FAILURE: Unapproved #[ignore] attributes detected in E2E tests"
    echo ""
    echo -e "$VIOLATIONS"
    echo "Options:"
    echo "  1. Remove #[ignore] and make the test run in CI"
    echo "  2. Add the file to the allowlist in tools/ci/lint-no-ignored-tests.sh"
    echo "     (requires coordinator approval — ignored tests are invisible to CI sweep)"
    echo ""
    echo "Rationale: Ignored tests create silent coverage gaps."
    exit 1
fi

echo "✅ Lint passed: No unapproved #[ignore] in E2E tests"
exit 0
