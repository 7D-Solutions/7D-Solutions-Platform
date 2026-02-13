#!/usr/bin/env bash
# Migration Boundary Checker
# Ensures SQL migrations do not reference tables from other modules

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Define all module migration directories
declare -A MODULES
MODULES[AUTH]="platform/identity-auth/db/migrations"
MODULES[AR]="modules/ar/db/migrations"
MODULES[SUBSCRIPTIONS]="modules/subscriptions/db/migrations"
MODULES[PAYMENTS]="modules/payments/db/migrations"
MODULES[NOTIFICATIONS]="modules/notifications/db/migrations"
MODULES[GL]="modules/gl/db/migrations"

# Define forbidden patterns for each module
# Pattern explanation:
# - REFERENCES <other_module_prefix>_<table> - Foreign keys to other modules
# - FROM <other_module_prefix>_<table> - Queries against other modules
# - JOIN <other_module_prefix>_<table> - Joins to other modules
# - schema <module_name> - Explicit schema references
# Excludes: Column names or table prefixes within the same module

declare -A FORBIDDEN_PATTERNS
FORBIDDEN_PATTERNS[AUTH]='(REFERENCES\s+(ar_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|FROM\s+(ar_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|JOIN\s+(ar_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|schema\s+(ar|subscriptions|payments|notifications|gl))'
FORBIDDEN_PATTERNS[AR]='(REFERENCES\s+(auth_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|FROM\s+(auth_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|JOIN\s+(auth_|subscriptions_|payments_|notifications_|journal_|accounts_|gl_)|schema\s+(auth|subscriptions|payments|notifications|gl))'
FORBIDDEN_PATTERNS[SUBSCRIPTIONS]='(REFERENCES\s+(auth_|ar_|payments_|notifications_|journal_|accounts_|gl_)|FROM\s+(auth_|ar_|payments_|notifications_|journal_|accounts_|gl_)|JOIN\s+(auth_|ar_|payments_|notifications_|journal_|accounts_|gl_)|schema\s+(auth|ar|payments|notifications|gl))'
FORBIDDEN_PATTERNS[PAYMENTS]='(REFERENCES\s+(auth_|ar_|subscriptions_|notifications_|journal_|accounts_|gl_)|FROM\s+(auth_|ar_|subscriptions_|notifications_|journal_|accounts_|gl_)|JOIN\s+(auth_|ar_|subscriptions_|notifications_|journal_|accounts_|gl_)|schema\s+(auth|ar|subscriptions|notifications|gl))'
FORBIDDEN_PATTERNS[NOTIFICATIONS]='(REFERENCES\s+(auth_|ar_|subscriptions_|payments_|journal_|accounts_|gl_)|FROM\s+(auth_|ar_|subscriptions_|payments_|journal_|accounts_|gl_)|JOIN\s+(auth_|ar_|subscriptions_|payments_|journal_|accounts_|gl_)|schema\s+(auth|ar|subscriptions|payments|gl))'
FORBIDDEN_PATTERNS[GL]='(REFERENCES\s+(auth_|ar_|subscriptions_|payments_|notifications_)|FROM\s+(auth_|ar_|subscriptions_|payments_|notifications_)|JOIN\s+(auth_|ar_|subscriptions_|payments_|notifications_)|schema\s+(auth|ar|subscriptions|payments|notifications))'

violations=0

echo "Checking migration boundaries for all modules..."

# Check if at least one module directory exists
found_any=false
for module in "${!MODULES[@]}"; do
    if [ -d "${MODULES[$module]}" ]; then
        found_any=true
        break
    fi
done

if [ "$found_any" = false ]; then
    echo -e "${YELLOW}Warning: No migration directories found${NC}"
    exit 0
fi

# Function to check for forbidden patterns
check_migration_file() {
    local file=$1
    local forbidden_pattern=$2
    local module_name=$3

    # Search for patterns (case-insensitive)
    matches=$(grep -inE "$forbidden_pattern" "$file" 2>/dev/null || true)

    if [ -n "$matches" ]; then
        echo -e "${RED}✗ Boundary violation in $file${NC}"
        echo -e "${RED}  $module_name migration references other module tables:${NC}"
        echo "$matches" | while IFS=: read -r line_num line_content; do
            echo -e "${RED}    Line $line_num: $(echo "$line_content" | xargs)${NC}"
        done
        echo ""
        ((violations++))
        return 1
    fi
    return 0
}

# Check each module's migrations for cross-module references
for module in "${!MODULES[@]}"; do
    migrations_dir="${MODULES[$module]}"

    if [ ! -d "$migrations_dir" ]; then
        echo -e "${YELLOW}Skipping $module (directory not found)${NC}"
        continue
    fi

    echo "Checking $module migrations for cross-module references..."

    for file in "$migrations_dir"/*.sql; do
        if [ -f "$file" ]; then
            forbidden_pattern="${FORBIDDEN_PATTERNS[$module]}"
            check_migration_file "$file" "$forbidden_pattern" "$module" || true
        fi
    done
done

if [ $violations -eq 0 ]; then
    echo -e "${GREEN}✓ No migration boundary violations detected${NC}"
    exit 0
else
    echo -e "${RED}✗ Found $violations migration boundary violation(s)${NC}"
    echo ""
    echo "Modules must not reference each other's tables in migrations."
    echo "Integration must happen via events and contracts only."
    exit 1
fi
