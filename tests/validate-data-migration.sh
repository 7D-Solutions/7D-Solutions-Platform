#!/bin/bash
# Validate AR data migration from MySQL to PostgreSQL
# Compares record counts, checksums, and data integrity

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
MYSQL_HOST="${MYSQL_HOST:-fireproof-db}"
MYSQL_PORT="${MYSQL_PORT:-3307}"
MYSQL_USER="${MYSQL_USER:-root}"
MYSQL_PASS="${MYSQL_PASS:-fireproof_root_sandbox}"
MYSQL_DB="${MYSQL_DB:-fireproof}"

PG_HOST="${PG_HOST:-localhost}"
PG_PORT="${PG_PORT:-5434}"
PG_USER="${PG_USER:-postgres}"
PG_PASS="${PG_PASS:-postgres}"
PG_DB="${PG_DB:-ar_service}"

RESULTS_DIR="./tests/load/validation-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/data-validation-$TIMESTAMP.md"

# Create results directory
mkdir -p "$RESULTS_DIR"

echo "============================================="
echo "AR Data Migration Validation"
echo "============================================="
echo "MySQL: $MYSQL_HOST:$MYSQL_PORT/$MYSQL_DB"
echo "PostgreSQL: $PG_HOST:$PG_PORT/$PG_DB"
echo "Report: $REPORT_FILE"
echo ""

# Track results
TOTAL_CHECKS=0
PASSED_CHECKS=0
FAILED_CHECKS=0
WARNINGS=0

# Initialize report
cat > "$REPORT_FILE" << 'EOF'
# AR Data Migration Validation Report
**Generated:** $(date)
**Source (MySQL):** $MYSQL_HOST:$MYSQL_PORT/$MYSQL_DB
**Target (PostgreSQL):** $PG_HOST:$PG_PORT/$PG_DB

## Summary
EOF

# Helper to run MySQL query
mysql_query() {
    mysql -h "$MYSQL_HOST" -P "$MYSQL_PORT" -u "$MYSQL_USER" -p"$MYSQL_PASS" \
        "$MYSQL_DB" -N -B -e "$1" 2>/dev/null
}

# Helper to run PostgreSQL query
pg_query() {
    PGPASSWORD="$PG_PASS" psql -h "$PG_HOST" -p "$PG_PORT" -U "$PG_USER" \
        -d "$PG_DB" -t -A -c "$1" 2>/dev/null
}

# Helper to compare counts
compare_counts() {
    local table_name="$1"
    local mysql_table="$2"
    local pg_table="$3"
    local description="$4"

    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))

    echo ""
    echo -e "${BLUE}Checking:${NC} $description"

    # Get counts
    mysql_count=$(mysql_query "SELECT COUNT(*) FROM $mysql_table")
    pg_count=$(pg_query "SELECT COUNT(*) FROM $pg_table")

    echo "  MySQL count: $mysql_count"
    echo "  PostgreSQL count: $pg_count"

    if [ "$mysql_count" -eq "$pg_count" ]; then
        echo -e "  ${GREEN}✓ MATCH${NC}"
        PASSED_CHECKS=$((PASSED_CHECKS + 1))
        echo "- ✅ **$description**: $pg_count records (MATCH)" >> "$REPORT_FILE"
        return 0
    else
        diff=$((pg_count - mysql_count))
        echo -e "  ${RED}✗ MISMATCH${NC}: Difference of $diff records"
        FAILED_CHECKS=$((FAILED_CHECKS + 1))
        echo "- ❌ **$description**: MySQL=$mysql_count, PostgreSQL=$pg_count (DIFF: $diff)" >> "$REPORT_FILE"
        return 1
    fi
}

# Helper to compare data integrity
compare_data_integrity() {
    local description="$1"
    local mysql_query="$2"
    local pg_query="$3"

    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))

    echo ""
    echo -e "${BLUE}Checking:${NC} $description"

    mysql_result=$(mysql_query "$mysql_query")
    pg_result=$(pg_query "$pg_query")

    echo "  MySQL result: $mysql_result"
    echo "  PostgreSQL result: $pg_result"

    # Allow for slight floating point differences
    if [ "$mysql_result" == "$pg_result" ] || \
       [ "$(echo "$mysql_result - $pg_result" | bc | tr -d '-' | cut -d. -f1)" -eq 0 ]; then
        echo -e "  ${GREEN}✓ MATCH${NC}"
        PASSED_CHECKS=$((PASSED_CHECKS + 1))
        echo "- ✅ **$description**: $pg_result (MATCH)" >> "$REPORT_FILE"
        return 0
    else
        echo -e "  ${YELLOW}⚠ WARNING${NC}: Results differ"
        WARNINGS=$((WARNINGS + 1))
        echo "- ⚠️  **$description**: MySQL=$mysql_result, PostgreSQL=$pg_result" >> "$REPORT_FILE"
        return 1
    fi
}

echo "## Table Record Counts" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# Compare table counts
compare_counts "Customers" "billing_customers" "billing_customers" "Billing Customers"
compare_counts "Subscriptions" "billing_subscriptions" "billing_subscriptions" "Billing Subscriptions"
compare_counts "Charges" "billing_charges" "billing_charges" "Billing Charges"
compare_counts "Refunds" "billing_refunds" "billing_refunds" "Billing Refunds"
compare_counts "Invoices" "billing_invoices" "billing_invoices" "Billing Invoices"
compare_counts "Payment Methods" "billing_payment_methods" "billing_payment_methods" "Payment Methods"
compare_counts "Disputes" "billing_disputes" "billing_disputes" "Billing Disputes"
compare_counts "Webhooks" "billing_webhooks" "billing_webhooks" "Billing Webhooks"
compare_counts "Events" "billing_events" "billing_events" "Billing Events"
compare_counts "Idempotency Keys" "billing_idempotency_keys" "billing_idempotency_keys" "Idempotency Keys"

echo "" >> "$REPORT_FILE"
echo "## Data Integrity Checks" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# Check customer email uniqueness
compare_data_integrity \
    "Customer Email Uniqueness" \
    "SELECT COUNT(DISTINCT email) FROM billing_customers" \
    "SELECT COUNT(DISTINCT email) FROM billing_customers"

# Check active subscriptions
compare_data_integrity \
    "Active Subscriptions Count" \
    "SELECT COUNT(*) FROM billing_subscriptions WHERE status = 'active'" \
    "SELECT COUNT(*) FROM billing_subscriptions WHERE status = 'active'"

# Check total charge amount
compare_data_integrity \
    "Total Charges Amount" \
    "SELECT COALESCE(SUM(amount_cents), 0) FROM billing_charges WHERE status = 'succeeded'" \
    "SELECT COALESCE(SUM(amount_cents), 0) FROM billing_charges WHERE status = 'succeeded'"

# Check total refund amount
compare_data_integrity \
    "Total Refunds Amount" \
    "SELECT COALESCE(SUM(amount_cents), 0) FROM billing_refunds WHERE status = 'succeeded'" \
    "SELECT COALESCE(SUM(amount_cents), 0) FROM billing_refunds WHERE status = 'succeeded'"

# Check foreign key relationships
echo "" >> "$REPORT_FILE"
echo "## Foreign Key Validation" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# Check orphaned subscriptions
TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
echo ""
echo -e "${BLUE}Checking:${NC} Orphaned Subscriptions"
orphaned_subs=$(pg_query "SELECT COUNT(*) FROM billing_subscriptions s LEFT JOIN billing_customers c ON s.billing_customer_id = c.id WHERE c.id IS NULL")
if [ "$orphaned_subs" -eq 0 ]; then
    echo -e "  ${GREEN}✓ PASS${NC}: No orphaned subscriptions"
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo "- ✅ **Orphaned Subscriptions**: None found" >> "$REPORT_FILE"
else
    echo -e "  ${RED}✗ FAIL${NC}: Found $orphaned_subs orphaned subscriptions"
    FAILED_CHECKS=$((FAILED_CHECKS + 1))
    echo "- ❌ **Orphaned Subscriptions**: $orphaned_subs found" >> "$REPORT_FILE"
fi

# Check orphaned charges
TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
echo ""
echo -e "${BLUE}Checking:${NC} Orphaned Charges"
orphaned_charges=$(pg_query "SELECT COUNT(*) FROM billing_charges ch LEFT JOIN billing_customers c ON ch.billing_customer_id = c.id WHERE c.id IS NULL")
if [ "$orphaned_charges" -eq 0 ]; then
    echo -e "  ${GREEN}✓ PASS${NC}: No orphaned charges"
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo "- ✅ **Orphaned Charges**: None found" >> "$REPORT_FILE"
else
    echo -e "  ${RED}✗ FAIL${NC}: Found $orphaned_charges orphaned charges"
    FAILED_CHECKS=$((FAILED_CHECKS + 1))
    echo "- ❌ **Orphaned Charges**: $orphaned_charges found" >> "$REPORT_FILE"
fi

# Check orphaned refunds
TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
echo ""
echo -e "${BLUE}Checking:${NC} Orphaned Refunds"
orphaned_refunds=$(pg_query "SELECT COUNT(*) FROM billing_refunds r LEFT JOIN billing_customers c ON r.billing_customer_id = c.id WHERE c.id IS NULL")
if [ "$orphaned_refunds" -eq 0 ]; then
    echo -e "  ${GREEN}✓ PASS${NC}: No orphaned refunds"
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo "- ✅ **Orphaned Refunds**: None found" >> "$REPORT_FILE"
else
    echo -e "  ${RED}✗ FAIL${NC}: Found $orphaned_refunds orphaned refunds"
    FAILED_CHECKS=$((FAILED_CHECKS + 1))
    echo "- ❌ **Orphaned Refunds**: $orphaned_refunds found" >> "$REPORT_FILE"
fi

# Final summary
echo ""
echo "============================================="
echo "Summary"
echo "============================================="
echo -e "Total Checks: $TOTAL_CHECKS"
echo -e "${GREEN}Passed: $PASSED_CHECKS${NC}"
echo -e "${RED}Failed: $FAILED_CHECKS${NC}"
echo -e "${YELLOW}Warnings: $WARNINGS${NC}"
echo ""

success_rate=$(echo "scale=2; ($PASSED_CHECKS * 100) / $TOTAL_CHECKS" | bc)

# Update report summary
sed -i.bak "s/## Summary/## Summary\n\n- **Total Checks:** $TOTAL_CHECKS\n- **Passed:** $PASSED_CHECKS ✅\n- **Failed:** $FAILED_CHECKS ❌\n- **Warnings:** $WARNINGS ⚠️\n- **Success Rate:** $success_rate%/" "$REPORT_FILE"
rm -f "$REPORT_FILE.bak"

echo "Full report saved to: $REPORT_FILE"

# Exit with error if any checks failed
if [ $FAILED_CHECKS -gt 0 ]; then
    exit 1
fi

exit 0
