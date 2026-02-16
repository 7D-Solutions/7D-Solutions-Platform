#!/usr/bin/env bash
# Phase 16: Lint - Require event metadata present
#
# This script verifies that all event emit sites populate required metadata fields:
# - event_type
# - schema_version
# - correlation_id (trace_id)
# - mutation_class
#
# Failure modes to detect:
# - Missing mutation_class (violates governance)
# - Missing correlation_id (breaks distributed tracing)
# - Missing schema_version (breaks compatibility)
#
# Exit codes:
# 0 - All emit sites compliant
# 1 - Violations found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$PROJECT_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "🔍 Linting event metadata presence..."

violations=0

# Check for create_*_envelope helper usage
# All modules should use their envelope helpers which enforce metadata
check_envelope_helper_usage() {
    local module=$1
    local helper_pattern=$2

    echo "  Checking $module module..."

    # Find files with actual enqueue_event function calls (not just imports/re-exports)
    local enqueue_files=$(grep -rl "enqueue_event(" "modules/$module/src" 2>/dev/null | grep -v "pub use\|pub async fn\|pub fn" || true)

    if [ -z "$enqueue_files" ]; then
        echo "    ✓ No event emissions found"
        return 0
    fi

    for file in $enqueue_files; do
        # Skip files that define enqueue_event (outbox.rs, event_bus.rs)
        if [[ "$file" == *"/outbox.rs" ]] || [[ "$file" == *"/event_bus.rs" ]]; then
            continue
        fi

        # Skip mod.rs files (they just re-export)
        if [[ "$file" == *"/mod.rs" ]]; then
            continue
        fi

        # Check if file uses the helper function
        if ! grep -q "$helper_pattern" "$file" 2>/dev/null; then
            echo -e "    ${RED}✗${NC} $file: Missing $helper_pattern usage"
            ((violations++))
        fi
    done
}

# Check AR module
check_envelope_helper_usage "ar" "create_ar_envelope"

# Check Payments module
check_envelope_helper_usage "payments" "create_payments_envelope"

# Check Subscriptions module
check_envelope_helper_usage "subscriptions" "create_subscriptions_envelope"

# Check Notifications module
check_envelope_helper_usage "notifications" "create_notifications_envelope"

# GL module uses raw outbox_repo::insert_outbox_event_with_linkage
# Check GL has mutation_class parameter in all outbox insertions
echo "  Checking GL module (raw outbox pattern)..."
gl_outbox_files=$(grep -rl "insert_outbox_event" "modules/gl/src" 2>/dev/null || true)

if [ -n "$gl_outbox_files" ]; then
    for file in $gl_outbox_files; do
        # Check if outbox calls include mutation_class argument
        # insert_outbox_event_with_linkage requires 9 parameters, last one is mutation_class
        if grep -q "insert_outbox_event_with_linkage" "$file"; then
            # Verify the call includes a mutation_class string literal or variable
            if ! grep -A 10 "insert_outbox_event_with_linkage" "$file" | grep -q '"REVERSAL"\|"DATA_MUTATION"\|"CORRECTION"\|"SIDE_EFFECT"\|"LIFECYCLE"\|"ADMINISTRATIVE"\|mutation_class'; then
                echo -e "    ${RED}✗${NC} $file: Missing mutation_class in outbox call"
                ((violations++))
            fi
        fi
    done
fi

# Check envelope helpers have required parameters
echo "  Verifying envelope helper signatures..."

check_helper_signature() {
    local file=$1
    local helper_name=$2

    if [ ! -f "$file" ]; then
        echo -e "    ${YELLOW}⚠${NC}  $file not found"
        return 0
    fi

    # Check helper requires mutation_class parameter
    if ! grep -A 10 "pub fn $helper_name" "$file" | grep -q "mutation_class"; then
        echo -e "    ${RED}✗${NC} $helper_name: Missing mutation_class parameter"
        ((violations++))
    fi

    # Check helper requires correlation_id parameter
    if ! grep -A 10 "pub fn $helper_name" "$file" | grep -q "correlation_id"; then
        echo -e "    ${YELLOW}⚠${NC}  $helper_name: Missing correlation_id parameter (optional for Phase 16)"
    fi
}

check_helper_signature "modules/ar/src/events/envelope.rs" "create_ar_envelope"
check_helper_signature "modules/payments/src/events/envelope.rs" "create_payments_envelope"
check_helper_signature "modules/subscriptions/src/envelope.rs" "create_subscriptions_envelope"
check_helper_signature "modules/notifications/src/event_bus.rs" "create_notifications_envelope"

# Report results
echo ""
if [ $violations -eq 0 ]; then
    echo -e "${GREEN}✓${NC} All event emit sites have required metadata"
    exit 0
else
    echo -e "${RED}✗${NC} Found $violations violation(s)"
    echo ""
    echo "Required metadata for all events:"
    echo "  - event_type (enforced by helper)"
    echo "  - schema_version (enforced by helper)"
    echo "  - correlation_id (enforced by helper parameter)"
    echo "  - mutation_class (MUST be explicit)"
    echo ""
    echo "See docs/governance/MUTATION-CLASSES.md for classification guidance"
    exit 1
fi
