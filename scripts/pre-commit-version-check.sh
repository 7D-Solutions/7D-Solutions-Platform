#!/usr/bin/env bash
# Gate 0: Secret Scan (gitleaks)
# Gate 1: Version Bump Enforcement
#
# Gate 0 runs gitleaks against staged changes before any other check.
# Gate 1 checks that commits to proven modules (version >= 1.0.0) include:
#   1. A version bump in Cargo.toml (or package.json)
#   2. A modification to REVISIONS.md
#
# Install: ln -sf ../../scripts/pre-commit-version-check.sh .git/hooks/pre-commit
# Standards: docs/security/vuln-triage.md (Gate 0), docs/VERSIONING.md (Gate 1)

set -uo pipefail

# ============================================
# Gate 0: Secret Scan (gitleaks)
# ============================================
_REPO_ROOT="$(git rev-parse --show-toplevel)"
_GITLEAKS_CONFIG="$_REPO_ROOT/.gitleaks.toml"

if ! command -v gitleaks >/dev/null 2>&1; then
    # Warn but do not block — a hook that fails on missing tools trains
    # developers to bypass it, which is worse than no hook at all.
    _OS="$(uname -s)"
    case "$_OS" in
        Darwin)
            _INSTALL_CMD="brew install gitleaks" ;;
        Linux)
            if command -v apt-get >/dev/null 2>&1; then
                _INSTALL_CMD="apt-get install gitleaks"
            else
                _INSTALL_CMD="see https://github.com/gitleaks/gitleaks#installing"
            fi
            ;;
        *)
            _INSTALL_CMD="see https://github.com/gitleaks/gitleaks#installing" ;;
    esac
    echo "⚠️  gitleaks not installed — secret scan skipped. To install: $_INSTALL_CMD" >&2
else
    if ! gitleaks protect --staged --redact --no-banner --config "$_GITLEAKS_CONFIG" 2>&1; then
        echo "" >&2
        echo "❌ Gate 0 FAILED: gitleaks detected a secret in staged changes." >&2
        echo "   Remove the secret before committing." >&2
        echo "   If this is a false positive, see docs/security/vuln-triage.md" >&2
        echo "   for how to add an allowlist entry (bead ID + reason required)." >&2
        echo "" >&2
        exit 1
    fi
fi

# Directories that contain deployable modules
MODULE_DIRS=("modules" "platform")

# Files that trigger the version check (under a module root)
TRIGGER_DIRS=("src/" "db/" "migrations/")

# ============================================
# Find all modules touched by this commit
# ============================================
# Get staged files (cached = staged for commit)
STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACMR 2>/dev/null)

if [ -z "$STAGED_FILES" ]; then
    exit 0
fi

# Collect unique module roots that have triggering changes
declare -A TOUCHED_MODULES

for file in $STAGED_FILES; do
    for dir in "${MODULE_DIRS[@]}"; do
        if [[ "$file" == "$dir/"* ]]; then
            # Extract module root: modules/ar/src/foo.rs → modules/ar
            module_root=$(echo "$file" | cut -d'/' -f1-2)

            # Check if the changed file is in a trigger directory
            remainder="${file#$module_root/}"
            for trigger in "${TRIGGER_DIRS[@]}"; do
                if [[ "$remainder" == "$trigger"* ]]; then
                    TOUCHED_MODULES["$module_root"]=1
                    break
                fi
            done
        fi
    done
done

# ============================================
# Gate: Service Catalog Freshness
# ============================================
# If any Cargo.toml or docker-compose file is staged, regenerate the catalog
# and check whether docs/PLATFORM-SERVICE-CATALOG.md needs updating.
CATALOG_TRIGGER=false
for file in $STAGED_FILES; do
    case "$file" in
        modules/*/Cargo.toml|platform/*/Cargo.toml|tools/*/Cargo.toml|\
        docker-compose.services.yml|docker-compose.data.yml)
            CATALOG_TRIGGER=true
            break
            ;;
    esac
done

if [ "$CATALOG_TRIGGER" = true ]; then
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    CATALOG="$REPO_ROOT/docs/PLATFORM-SERVICE-CATALOG.md"
    GENERATOR="$REPO_ROOT/scripts/generate-service-catalog.sh"

    if [ -x "$GENERATOR" ]; then
        # Save current catalog, regenerate, compare
        BEFORE=$(cat "$CATALOG" 2>/dev/null || true)
        bash "$GENERATOR" >/dev/null 2>&1
        AFTER=$(cat "$CATALOG" 2>/dev/null || true)

        if [ "$BEFORE" != "$AFTER" ]; then
            echo "" >&2
            echo "❌ Service catalog is stale." >&2
            echo "   The catalog has been regenerated for you." >&2
            echo "   Stage it:  git add docs/PLATFORM-SERVICE-CATALOG.md" >&2
            echo "" >&2
            exit 1
        fi
    fi
fi

# ============================================
# Gate 1: Version Bump Enforcement
# ============================================
# shellcheck disable=SC2128
if [ -z "${TOUCHED_MODULES[*]:-}" ]; then
    exit 0
fi

ERRORS=()

for module_root in "${!TOUCHED_MODULES[@]}"; do
    # Determine package file
    PACKAGE_FILE=""
    if [ -f "$module_root/Cargo.toml" ]; then
        PACKAGE_FILE="$module_root/Cargo.toml"
    elif [ -f "$module_root/package.json" ]; then
        PACKAGE_FILE="$module_root/package.json"
    else
        continue  # No package file, skip
    fi

    # Get current version from the working tree / staged content
    if [[ "$PACKAGE_FILE" == *"Cargo.toml" ]]; then
        # Use staged content if available, else working tree
        CURRENT_VERSION=$(git show :"$PACKAGE_FILE" 2>/dev/null | grep -m1 '^version' | sed 's/.*"\(.*\)".*/\1/')
    else
        CURRENT_VERSION=$(git show :"$PACKAGE_FILE" 2>/dev/null | jq -r '.version // empty')
    fi

    if [ -z "$CURRENT_VERSION" ]; then
        continue  # Can't determine version, skip
    fi

    # Parse major version
    MAJOR=$(echo "$CURRENT_VERSION" | cut -d'.' -f1)

    # If major < 1, module is unproven — skip
    if [ "$MAJOR" -lt 1 ] 2>/dev/null; then
        continue
    fi

    # Module is proven (>= 1.0.0). Check for version bump.
    # Get the version from HEAD (before this commit)
    if [[ "$PACKAGE_FILE" == *"Cargo.toml" ]]; then
        HEAD_VERSION=$(git show HEAD:"$PACKAGE_FILE" 2>/dev/null | grep -m1 '^version' | sed 's/.*"\(.*\)".*/\1/')
    else
        HEAD_VERSION=$(git show HEAD:"$PACKAGE_FILE" 2>/dev/null | jq -r '.version // empty')
    fi

    # If this is a new file (no HEAD version), it's being created — allow
    if [ -z "$HEAD_VERSION" ]; then
        continue
    fi

    # Check 1: Was the version bumped?
    VERSION_BUMPED=false
    if echo "$STAGED_FILES" | grep -q "^${PACKAGE_FILE}$"; then
        if [ "$CURRENT_VERSION" != "$HEAD_VERSION" ]; then
            VERSION_BUMPED=true
        fi
    fi

    # Check 2: Was REVISIONS.md modified?
    REVISIONS_FILE="$module_root/REVISIONS.md"
    REVISIONS_MODIFIED=false
    if echo "$STAGED_FILES" | grep -q "^${REVISIONS_FILE}$"; then
        REVISIONS_MODIFIED=true
    fi

    # Report errors
    MODULE_NAME=$(basename "$module_root")
    if [ "$VERSION_BUMPED" = false ]; then
        ERRORS+=("  ✗ $MODULE_NAME ($module_root): Version not bumped. Current: $HEAD_VERSION. Bump in $PACKAGE_FILE.")
    fi
    if [ "$REVISIONS_MODIFIED" = false ]; then
        ERRORS+=("  ✗ $MODULE_NAME ($module_root): REVISIONS.md not updated. Add a revision entry to $REVISIONS_FILE.")
    fi
done

# ============================================
# Report
# ============================================
if [ ${#ERRORS[@]} -gt 0 ]; then
    echo "" >&2
    echo "❌ Gate 1 FAILED: Proven module(s) changed without version bump / revision entry" >&2
    echo "" >&2
    echo "Standard: docs/VERSIONING.md" >&2
    echo "" >&2
    for err in "${ERRORS[@]}"; do
        echo "$err" >&2
    done
    echo "" >&2
    echo "Fix: Bump the version in the package file and add a row to REVISIONS.md." >&2
    echo "     Both must be in the same commit as the code change." >&2
    echo "" >&2
    exit 1
fi

exit 0
