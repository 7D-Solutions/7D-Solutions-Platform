#!/usr/bin/env bash
# scripts/e2e_run.sh — E2E test runner with tag filtering
#
# Usage:
#   ./scripts/e2e_run.sh                     # Run all E2E tests
#   ./scripts/e2e_run.sh --tag phase42-smoke # Run only tests with this tag
#   ./scripts/e2e_run.sh --tag smoke         # Run all smoke tests
#   ./scripts/e2e_run.sh --list              # List available tests and tags
#   ./scripts/e2e_run.sh --list --tag smoke  # List tests matching a tag
#
# Environment:
#   E2E_TIMEOUT=30  — seconds to wait for each service readiness (default: 30)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_DIR="$PROJECT_ROOT/tests/e2e"
HELPERS="$E2E_DIR/lib/helpers.sh"

# ============================================================================
# Argument Parsing
# ============================================================================

TAG_FILTER=""
LIST_MODE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag)
            TAG_FILTER="$2"
            shift 2
            ;;
        --list)
            LIST_MODE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--tag TAG] [--list]"
            echo ""
            echo "Options:"
            echo "  --tag TAG   Run only tests declaring this tag"
            echo "  --list      List tests (and their tags) instead of running"
            echo ""
            echo "Tags are declared in test scripts as: # TAGS: tag1 tag2 tag3"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# ============================================================================
# Discover Test Scripts
# ============================================================================

# Find all .sh files under tests/e2e/ excluding lib/
discover_tests() {
    find "$E2E_DIR" -name '*.sh' -not -path '*/lib/*' | sort
}

# Filter tests by tag (if tag filter is set)
filter_by_tag() {
    local tag="$1"
    while IFS= read -r test_file; do
        if [[ -z "$tag" ]]; then
            echo "$test_file"
        else
            local tags
            tags=$(grep -m1 '^# TAGS:' "$test_file" 2>/dev/null | sed 's/^# TAGS: *//' || true)
            if [[ " $tags " == *" $tag "* ]]; then
                echo "$test_file"
            fi
        fi
    done
}

# ============================================================================
# List Mode
# ============================================================================

if [[ "$LIST_MODE" == "true" ]]; then
    echo "=== E2E Tests ==="
    if [[ -n "$TAG_FILTER" ]]; then
        echo "Filter: --tag $TAG_FILTER"
    fi
    echo ""

    discover_tests | filter_by_tag "$TAG_FILTER" | while IFS= read -r test_file; do
        local_path="${test_file#$PROJECT_ROOT/}"
        tags=$(grep -m1 '^# TAGS:' "$test_file" 2>/dev/null | sed 's/^# TAGS: *//' || echo "(none)")
        echo "  $local_path"
        echo "    tags: $tags"
    done
    exit 0
fi

# ============================================================================
# Run Mode
# ============================================================================

# Source helpers to get counters and reporting functions
source "$HELPERS"

echo "=== E2E Test Runner ==="
if [[ -n "$TAG_FILTER" ]]; then
    echo "Tag filter: $TAG_FILTER"
fi
echo ""

# Collect matching test files
mapfile -t TEST_FILES < <(discover_tests | filter_by_tag "$TAG_FILTER")

if [[ ${#TEST_FILES[@]} -eq 0 ]]; then
    echo "No tests found matching criteria."
    exit 0
fi

echo "Found ${#TEST_FILES[@]} test(s) to run."
echo ""

# Run each test script
for test_file in "${TEST_FILES[@]}"; do
    local_path="${test_file#$PROJECT_ROOT/}"
    echo "--- $local_path ---"

    # Each test script sources helpers.sh itself, so it can use e2e_pass/fail.
    # We run it in the same process (source) to accumulate counters.
    source "$test_file"

    echo ""
done

# Print summary
e2e_summary
