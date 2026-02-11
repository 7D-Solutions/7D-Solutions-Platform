#!/usr/bin/env bash
# Migration Boundary Checker
# Ensures SQL migrations do not reference tables from other modules

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

AUTH_MIGRATIONS="platform/identity-auth/db/migrations"
AR_MIGRATIONS="modules/ar/db/migrations"

violations=0

echo "Checking migration boundaries..."

# Check if directories exist
if [ ! -d "$AUTH_MIGRATIONS" ]; then
    echo -e "${YELLOW}Warning: $AUTH_MIGRATIONS not found, skipping AUTH checks${NC}"
    AUTH_MIGRATIONS=""
fi

if [ ! -d "$AR_MIGRATIONS" ]; then
    echo -e "${YELLOW}Warning: $AR_MIGRATIONS not found, skipping AR checks${NC}"
    AR_MIGRATIONS=""
fi

# If both missing, warn and exit success
if [ -z "$AUTH_MIGRATIONS" ] && [ -z "$AR_MIGRATIONS" ]; then
    echo -e "${YELLOW}Warning: No migration directories found${NC}"
    exit 0
fi

# Function to check for forbidden patterns
check_migration_file() {
    local file=$1
    local forbidden_pattern=$2
    local module_name=$3
    local forbidden_module=$4
    
    # Search for patterns (case-insensitive)
    matches=$(grep -inE "$forbidden_pattern" "$file" 2>/dev/null || true)
    
    if [ -n "$matches" ]; then
        echo -e "${RED}✗ Boundary violation in $file${NC}"
        echo -e "${RED}  $module_name migration references $forbidden_module tables/schema:${NC}"
        echo "$matches" | while IFS=: read -r line_num line_content; do
            echo -e "${RED}    Line $line_num: $(echo "$line_content" | xargs)${NC}"
        done
        echo ""
        ((violations++))
        return 1
    fi
    return 0
}

# Check AUTH migrations for AR references
if [ -n "$AUTH_MIGRATIONS" ]; then
    echo "Checking AUTH migrations for AR table references..."
    for file in "$AUTH_MIGRATIONS"/*.sql; do
        if [ -f "$file" ]; then
            # Patterns that indicate AR module references
            # \bar_ - word boundary + ar_
            # schema ar - explicit schema reference
            # from ar\. - qualified table reference
            check_migration_file "$file" '\b(ar_[a-z_]+|schema\s+ar|from\s+ar\.|join\s+ar\.|into\s+ar\.)' "AUTH" "AR" || true
        fi
    done
fi

# Check AR migrations for AUTH references
if [ -n "$AR_MIGRATIONS" ]; then
    echo "Checking AR migrations for AUTH table references..."
    for file in "$AR_MIGRATIONS"/*.sql; do
        if [ -f "$file" ]; then
            # Patterns that indicate AUTH module references
            check_migration_file "$file" '\b(auth_[a-z_]+|schema\s+auth|from\s+auth\.|join\s+auth\.|into\s+auth\.)' "AR" "AUTH" || true
        fi
    done
fi

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
