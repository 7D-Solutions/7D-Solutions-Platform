#!/usr/bin/env bash
# lint_revisions.sh — Validate REVISIONS.md completeness for all proven modules.
#
# A "proven module" is any module whose package file (Cargo.toml or package.json)
# has a version >= 1.0.0. Unproven modules (v0.x.x) are skipped.
#
# Usage:
#   bash scripts/versioning/lint_revisions.sh [--module <module-path>]
#
# Options:
#   --module <path>  Lint a single module directory instead of all modules.
#
# Exit codes:
#   0 — All proven modules have valid, complete REVISIONS.md entries.
#   1 — One or more proven modules failed validation.
#
# Required fields per REVISIONS.md entry (docs/VERSIONING.md):
#
#   Field              | Column       | Valid value
#   -------------------+--------------+----------------------------------------------
#   version            | Version      | SemVer matching the package file (>= 1.0.0)
#   date               | Date         | ISO date (YYYY-MM-DD), not the placeholder text
#   bead               | Bead         | Non-empty, not the placeholder "bd-xxxx"
#   summary            | What Changed | Non-empty, does not start with "TODO"
#   why                | Why          | Non-empty, does not start with "TODO"
#   compatibility      | Breaking?    | Non-empty (use "No" or "YES: <notes>")
#
# Proof command convention (not enforced here — enforced by Gate 1 pre-commit hook):
#   Before a module is promoted to 1.0.0, a proof script must exist at:
#     scripts/proof_{module_name}.sh
#   See docs/VERSIONING.md § "When proving a module for the first time."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

SINGLE_MODULE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --module)
            SINGLE_MODULE="${2:-}"
            shift 2
            ;;
        -h|--help)
            sed -n 's/^# \{0,1\}//p' "$0" | head -40
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log()  { echo "[lint-revisions] $*"; }
warn() { echo "[lint-revisions] WARN: $*" >&2; }
fail() { echo "[lint-revisions] FAIL: $*" >&2; FAILURES=$((FAILURES + 1)); }

FAILURES=0

# Extract version from a Cargo.toml (first [package] version line)
cargo_version() {
    local toml="$1"
    # Match 'version = "X.Y.Z"' under [package] section using grep + sed (portable)
    awk '
        /^\[package\]/ { in_pkg=1; next }
        /^\[/ { in_pkg=0 }
        in_pkg && /^version[[:space:]]*=/ { print; exit }
    ' "$toml" | sed 's/.*"\([^"]*\)".*/\1/'
}

# Extract version from a package.json "version" field
npm_version() {
    local pkgjson="$1"
    python3 -c "import json,sys; d=json.load(open('$pkgjson')); print(d.get('version',''))"
}

# Compare version strings: is_proven returns 0 if version >= 1.0.0
is_proven() {
    local ver="$1"
    local major
    major=$(echo "$ver" | cut -d. -f1)
    [[ "$major" -ge 1 ]]
}

# ---------------------------------------------------------------------------
# REVISIONS.md validation for a single module
# ---------------------------------------------------------------------------

validate_module() {
    local module_dir="$1"
    local version="$2"
    local module_name
    module_name=$(basename "$module_dir")

    log "Checking $module_dir (v$version) ..."

    local revisions_file="$module_dir/REVISIONS.md"

    # --- Check 1: REVISIONS.md must exist ---
    if [[ ! -f "$revisions_file" ]]; then
        fail "$module_dir: REVISIONS.md is missing (module is proven at v$version)"
        return
    fi

    # --- Check 2: must have a row for the current version ---
    # Table rows start with "| <version> |" where version matches exactly
    local escaped_version
    escaped_version=$(echo "$version" | sed 's/\./\\./g')

    if ! grep -qE "^\| $escaped_version[[:space:]]*\|" "$revisions_file"; then
        fail "$module_dir: REVISIONS.md has no entry for current version $version"
        fail "  Add a row with: bash scripts/versioning/new_revision_entry.sh $module_dir $version"
        return
    fi

    # --- Check 3: validate each field in the version row ---
    # Parse the matching row. Table format: | Version | Date | Bead | What Changed | Why | Breaking? |
    local row
    row=$(grep -E "^\| $escaped_version[[:space:]]*\|" "$revisions_file" | head -1)

    # Split into fields by '|' — field indices (0-based after split):
    #   0: empty (before first |)
    #   1: Version
    #   2: Date
    #   3: Bead
    #   4: What Changed
    #   5: Why
    #   6: Breaking?
    #   7: empty (after last |)
    validate_row "$module_dir" "$version" "$row"
}

validate_row() {
    local module_dir="$1"
    local version="$2"
    local row="$3"

    # Use Python for reliable pipe-splitting (fields may contain spaces)
    python3 - "$module_dir" "$version" "$row" <<'PYEOF'
import sys

module_dir = sys.argv[1]
version    = sys.argv[2]
row        = sys.argv[3]

# Split on '|' and strip whitespace
fields = [f.strip() for f in row.split('|')]
# fields[0] = '' (before first |), fields[1..6] = content, fields[7] = '' (after last |)

if len(fields) < 7:
    print(f"FAIL: {module_dir}: malformed row for v{version} — expected 6 columns, got {len(fields)-2}", file=sys.stderr)
    print(f"  Row: {row}", file=sys.stderr)
    sys.exit(1)

row_version    = fields[1]
row_date       = fields[2]
row_bead       = fields[3]
row_what       = fields[4]
row_why        = fields[5]
row_breaking   = fields[6] if len(fields) > 6 else ''

errors = []

# Version field
if not row_version:
    errors.append("Version column is empty")

# Date field — must be non-empty and not the literal placeholder "YYYY-MM-DD"
if not row_date:
    errors.append("Date column is empty")
elif row_date == "YYYY-MM-DD":
    errors.append(f"Date column is still a placeholder: '{row_date}'")

# Bead field — must not be empty or "bd-xxxx"
if not row_bead:
    errors.append("Bead column is empty")
elif row_bead == "bd-xxxx":
    errors.append("Bead column is still a placeholder: 'bd-xxxx'")

# What Changed — must not be empty or start with "TODO"
if not row_what:
    errors.append("What Changed column is empty")
elif row_what.upper().startswith("TODO"):
    errors.append(f"What Changed column still has a TODO placeholder: '{row_what[:60]}...'")

# Why — must not be empty or start with "TODO"
if not row_why:
    errors.append("Why column is empty")
elif row_why.upper().startswith("TODO"):
    errors.append(f"Why column still has a TODO placeholder: '{row_why[:60]}...'")

# Breaking? — must not be empty
if not row_breaking:
    errors.append("Breaking? column is empty (use 'No' or 'YES: <migration note>')")

if errors:
    print(f"[lint-revisions] FAIL: {module_dir} v{version}:", file=sys.stderr)
    for e in errors:
        print(f"  - {e}", file=sys.stderr)
    sys.exit(1)
else:
    print(f"[lint-revisions]   ✓ v{version}: all required fields present")
PYEOF

    local exit_code=$?
    if [[ $exit_code -ne 0 ]]; then
        FAILURES=$((FAILURES + 1))
    fi
}

# ---------------------------------------------------------------------------
# Discover and scan modules
# ---------------------------------------------------------------------------

scan_module() {
    local module_dir="$1"
    local version=""

    # Detect version source
    if [[ -f "$module_dir/Cargo.toml" ]]; then
        version=$(cargo_version "$module_dir/Cargo.toml")
    elif [[ -f "$module_dir/package.json" ]]; then
        version=$(npm_version "$module_dir/package.json")
    else
        return  # Not a module we track
    fi

    if [[ -z "$version" ]]; then
        warn "$module_dir: could not read version — skipping"
        return
    fi

    if is_proven "$version"; then
        validate_module "$module_dir" "$version"
    else
        log "Skipping $module_dir (v$version is unproven — 0.x.x)"
    fi
}

echo "=== REVISIONS.md Lint ==="
echo ""

if [[ -n "$SINGLE_MODULE" ]]; then
    scan_module "$REPO_ROOT/$SINGLE_MODULE"
else
    # Scan all module directories
    for dir in \
        "$REPO_ROOT"/modules/*/ \
        "$REPO_ROOT"/platform/*/ \
        "$REPO_ROOT"/apps/*/
    do
        [[ -d "$dir" ]] || continue
        scan_module "${dir%/}"
    done
fi

echo ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

if [[ $FAILURES -eq 0 ]]; then
    echo "✓ REVISIONS.md lint passed — all proven modules have valid entries."
    exit 0
else
    echo "✗ REVISIONS.md lint FAILED — $FAILURES error(s) found." >&2
    echo "" >&2
    echo "Fix:" >&2
    echo "  1. Run: bash scripts/versioning/new_revision_entry.sh <module> <version> <bead>" >&2
    echo "  2. Fill in the TODO placeholders in the generated row." >&2
    echo "  3. Commit version bump + REVISIONS.md row in the same commit." >&2
    echo "" >&2
    echo "See docs/VERSIONING.md for the full standard." >&2
    exit 1
fi
