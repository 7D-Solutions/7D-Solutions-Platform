#!/usr/bin/env bash
# lint-file-size.sh — Enforce max LOC per source file
#
# Exit codes:
#   0 = all files pass (allowlisted files produce warnings only)
#   1 = at least one non-allowlisted file exceeds ERROR threshold
#
# Thresholds:
#   >500 LOC  = WARNING (allowlisted files get a note instead)
#   >1000 LOC = ERROR   (unless on allowlist)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

WARN_THRESHOLD=500
ERROR_THRESHOLD=1000
ALLOWLIST_FILE="$PROJECT_ROOT/.file-size-allowlist"

# Counters
total=0
warnings=0
errors=0
allowlisted=0

# Load allowlist (strip comments and blank lines)
declare -A allowlist
if [[ -f "$ALLOWLIST_FILE" ]]; then
    while IFS= read -r line; do
        # Skip comments and blank lines
        line="${line%%#*}"
        line="$(echo "$line" | xargs)"
        [[ -z "$line" ]] && continue
        allowlist["$line"]=1
    done < "$ALLOWLIST_FILE"
fi

# Find all .rs source files in modules/ and platform/
# Exclude: target dirs, test files, e2e test files
while IFS= read -r file; do
    # Skip test files
    basename="$(basename "$file")"
    [[ "$basename" == *_test.rs ]] && continue
    [[ "$basename" == *_e2e.rs ]] && continue

    # Get relative path from project root
    relpath="${file#$PROJECT_ROOT/}"

    # Skip paths containing /target/ or /tests/
    [[ "$relpath" == */target/* ]] && continue
    [[ "$relpath" == */tests/* ]] && continue

    loc=$(wc -l < "$file")
    total=$((total + 1))

    if (( loc > ERROR_THRESHOLD )); then
        if [[ -n "${allowlist[$relpath]:-}" ]]; then
            printf "  ALLOWLISTED  %4d LOC  %s\n" "$loc" "$relpath"
            allowlisted=$((allowlisted + 1))
        else
            printf "  ERROR        %4d LOC  %s\n" "$loc" "$relpath"
            errors=$((errors + 1))
        fi
    elif (( loc > WARN_THRESHOLD )); then
        if [[ -n "${allowlist[$relpath]:-}" ]]; then
            printf "  ALLOWLISTED  %4d LOC  %s\n" "$loc" "$relpath"
            allowlisted=$((allowlisted + 1))
        else
            printf "  WARNING      %4d LOC  %s\n" "$loc" "$relpath"
            warnings=$((warnings + 1))
        fi
    fi
done < <(find "$PROJECT_ROOT/modules" "$PROJECT_ROOT/platform" -name '*.rs' -type f 2>/dev/null)

echo ""
echo "=== File Size Lint Summary ==="
echo "  Files checked:  $total"
echo "  Warnings:       $warnings  (>$WARN_THRESHOLD LOC, not allowlisted)"
echo "  Errors:         $errors  (>$ERROR_THRESHOLD LOC, not allowlisted)"
echo "  Allowlisted:    $allowlisted"
echo ""

if (( errors > 0 )); then
    echo "FAILED: $errors file(s) exceed $ERROR_THRESHOLD LOC without allowlist entry."
    echo "Either split the file or add it to .file-size-allowlist with a tracking bead."
    exit 1
fi

if (( warnings > 0 )); then
    echo "PASSED with warnings: $warnings file(s) exceed $WARN_THRESHOLD LOC."
    echo "Consider splitting these files into logical modules."
fi

if (( warnings == 0 && errors == 0 && allowlisted == 0 )); then
    echo "PASSED: All files under $WARN_THRESHOLD LOC."
fi

if (( warnings == 0 && errors == 0 && allowlisted > 0 )); then
    echo "PASSED: $allowlisted file(s) on allowlist. Burn down as refactoring lands."
fi

exit 0
