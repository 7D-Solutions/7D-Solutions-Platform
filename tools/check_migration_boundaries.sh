#!/usr/bin/env bash
# Migration Boundary Checker
# Ensures SQL migrations do not reference tables from other modules.
# Auto-discovers all modules from the filesystem.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Shared table names that multiple modules legitimately create independently.
# These are per-module copies, not cross-module references.
SHARED_TABLES="events_outbox processed_events failed_events idempotency_keys"

# Auto-discover all migration directories
declare -a MODULE_NAMES=()
declare -a MODULE_DIRS=()

for mig_dir in modules/*/db/migrations platform/*/db/migrations; do
    if [ -d "$mig_dir" ]; then
        # Extract module name from path (e.g., modules/ar/db/migrations -> ar)
        mod_path="${mig_dir%/db/migrations}"
        mod_name=$(basename "$mod_path")
        MODULE_NAMES+=("$mod_name")
        MODULE_DIRS+=("$mig_dir")
    fi
done

if [ ${#MODULE_NAMES[@]} -eq 0 ]; then
    echo -e "${YELLOW}Warning: No migration directories found${NC}"
    exit 0
fi

echo "Checking migration boundaries for ${#MODULE_NAMES[@]} modules..."

# Step 1: Collect table names created by each module
declare -A MODULE_TABLES
for i in "${!MODULE_NAMES[@]}"; do
    mod="${MODULE_NAMES[$i]}"
    dir="${MODULE_DIRS[$i]}"
    # Extract table names from CREATE TABLE statements
    tables=$(grep -ohiE 'CREATE TABLE\s+(IF NOT EXISTS\s+)?([a-z_][a-z0-9_]+)' "$dir"/*.sql 2>/dev/null \
        | sed -E 's/CREATE TABLE\s+(IF NOT EXISTS\s+)?//' \
        | sort -u \
        | tr '\n' ' ')
    MODULE_TABLES[$mod]="$tables"
done

# Step 2: For each module, check its migrations don't REFERENCES/FROM/JOIN tables owned by other modules
violations=0

for i in "${!MODULE_NAMES[@]}"; do
    current_mod="${MODULE_NAMES[$i]}"
    current_dir="${MODULE_DIRS[$i]}"

    # Build a list of forbidden table names: all tables from OTHER modules
    forbidden_tables=()
    for j in "${!MODULE_NAMES[@]}"; do
        if [ "$i" = "$j" ]; then
            continue
        fi
        for table in ${MODULE_TABLES[${MODULE_NAMES[$j]}]}; do
            # Skip shared infrastructure tables
            is_shared=false
            for shared in $SHARED_TABLES; do
                if [ "$table" = "$shared" ]; then
                    is_shared=true
                    break
                fi
            done
            if [ "$is_shared" = false ]; then
                forbidden_tables+=("$table")
            fi
        done
    done

    if [ ${#forbidden_tables[@]} -eq 0 ]; then
        continue
    fi

    # Build a single grep pattern: (REFERENCES|FROM|JOIN)\s+<table_name>
    # We check for these keywords followed by a foreign table name
    pattern_parts=()
    for table in "${forbidden_tables[@]}"; do
        pattern_parts+=("$table")
    done

    # Join table names with | for alternation
    tables_alt=$(printf '%s\n' "${pattern_parts[@]}" | sort -u | paste -sd'|' -)
    pattern="(REFERENCES|FROM|JOIN)[[:space:]]+(${tables_alt})[[:space:]]*[^a-z0-9_]"

    # Check each migration file
    for file in "$current_dir"/*.sql; do
        if [ ! -f "$file" ]; then
            continue
        fi
        matches=$(grep -inE "$pattern" "$file" 2>/dev/null || true)
        if [ -n "$matches" ]; then
            echo -e "${RED}✗ Boundary violation in $file${NC}"
            echo -e "${RED}  $current_mod migration references other module tables:${NC}"
            echo "$matches" | head -5 | while IFS= read -r line; do
                echo -e "${RED}    $line${NC}"
            done
            echo ""
            ((violations++)) || true
        fi
    done
done

if [ $violations -eq 0 ]; then
    echo -e "${GREEN}✓ No migration boundary violations detected (${#MODULE_NAMES[@]} modules checked)${NC}"
    exit 0
else
    echo -e "${RED}✗ Found $violations migration boundary violation(s)${NC}"
    echo ""
    echo "Modules must not reference each other's tables in migrations."
    echo "Integration must happen via events and contracts only."
    exit 1
fi
