#!/bin/bash
# Complete AR migration validation suite
# Runs all tests and generates comprehensive report

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="./test-results"
REPORT_FILE="$RESULTS_DIR/validation-suite-$TIMESTAMP.md"

# Create results directory
mkdir -p "$RESULTS_DIR"

echo "============================================="
echo "AR Migration Validation Suite"
echo "============================================="
echo "Timestamp: $TIMESTAMP"
echo "Report: $REPORT_FILE"
echo ""

# Initialize report
cat > "$REPORT_FILE" << EOF
# AR Migration Validation Suite Results
**Generated:** $(date)
**Timestamp:** $TIMESTAMP

## Test Execution Summary
EOF

# Track overall results
TOTAL_SUITES=0
PASSED_SUITES=0
FAILED_SUITES=0

# Function to run test suite and capture results
run_test_suite() {
    local suite_name="$1"
    local test_file="$2"

    TOTAL_SUITES=$((TOTAL_SUITES + 1))

    echo ""
    echo "==========================================="
    echo -e "${BLUE}Running:${NC} $suite_name"
    echo "==========================================="

    # Run test and capture output
    output_file="$RESULTS_DIR/${test_file//\//_}-$TIMESTAMP.txt"

    if cargo test --test "$test_file" 2>&1 | tee "$output_file"; then
        echo -e "${GREEN}✓ PASSED${NC}: $suite_name"
        PASSED_SUITES=$((PASSED_SUITES + 1))

        # Extract test results
        test_result=$(grep "test result:" "$output_file" || echo "test result: unknown")

        echo "" >> "$REPORT_FILE"
        echo "### ✅ $suite_name" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"
        echo "$test_result" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"
    else
        echo -e "${RED}✗ FAILED${NC}: $suite_name"
        FAILED_SUITES=$((FAILED_SUITES + 1))

        # Extract test results
        test_result=$(grep "test result:" "$output_file" || echo "test result: unknown")

        echo "" >> "$REPORT_FILE"
        echo "### ❌ $suite_name" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"
        echo "$test_result" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"

        # Extract failures
        echo "" >> "$REPORT_FILE"
        echo "**Failures:**" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"
        grep "panicked at" "$output_file" | head -20 >> "$REPORT_FILE" || echo "No panic details found" >> "$REPORT_FILE"
        echo "\`\`\`" >> "$REPORT_FILE"
    fi
}

# Check environment
echo -e "${BLUE}Checking environment...${NC}"
if [ -z "$DATABASE_URL" ] && [ -z "$DATABASE_URL_AR" ]; then
    echo -e "${YELLOW}⚠ WARNING:${NC} DATABASE_URL or DATABASE_URL_AR not set"
    echo "Set DATABASE_URL_AR=postgresql://user:pass@localhost:5436/ar_db"
fi

# Check if services are running
echo -e "${BLUE}Checking services...${NC}"
if ! docker ps | grep -q "7d-ar-backend"; then
    echo -e "${YELLOW}⚠ WARNING:${NC} 7d-ar-backend not running"
fi

if ! docker ps | grep -q "7d-ar-postgres"; then
    echo -e "${YELLOW}⚠ WARNING:${NC} 7d-ar-postgres not running"
fi

echo "" >> "$REPORT_FILE"
echo "## Environment" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"
echo "- **Rust version:** $(rustc --version)" >> "$REPORT_FILE"
echo "- **Cargo version:** $(cargo --version)" >> "$REPORT_FILE"
echo "- **Database:** ${DATABASE_URL_AR:-Not set}" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# Run all test suites
echo "" >> "$REPORT_FILE"
echo "## Unit Tests" >> "$REPORT_FILE"

echo ""
echo -e "${BLUE}Running unit tests...${NC}"
if cargo test --lib 2>&1 | tee "$RESULTS_DIR/unit-tests-$TIMESTAMP.txt"; then
    unit_result=$(grep "test result:" "$RESULTS_DIR/unit-tests-$TIMESTAMP.txt" || echo "test result: unknown")
    echo "" >> "$REPORT_FILE"
    echo "### ✅ Unit Tests (lib)" >> "$REPORT_FILE"
    echo "\`\`\`" >> "$REPORT_FILE"
    echo "$unit_result" >> "$REPORT_FILE"
    echo "\`\`\`" >> "$REPORT_FILE"
else
    echo -e "${RED}✗ FAILED${NC}: Unit tests"
fi

echo "" >> "$REPORT_FILE"
echo "## Integration Tests" >> "$REPORT_FILE"

# Run integration test suites
run_test_suite "Customer Tests" "customer_tests"
run_test_suite "Subscription Tests" "subscription_tests"
run_test_suite "Payment Tests" "payment_tests"
run_test_suite "Webhook Tests" "webhook_tests"
run_test_suite "Idempotency Tests" "idempotency_test"

echo "" >> "$REPORT_FILE"
echo "## End-to-End Workflow Tests" >> "$REPORT_FILE"

run_test_suite "E2E Workflows" "e2e_workflows"

# Summary
echo ""
echo "============================================="
echo "Validation Suite Summary"
echo "============================================="
echo -e "Total Test Suites: $TOTAL_SUITES"
echo -e "${GREEN}Passed: $PASSED_SUITES${NC}"
echo -e "${RED}Failed: $FAILED_SUITES${NC}"

if [ $FAILED_SUITES -gt 0 ]; then
    success_rate=$(echo "scale=2; ($PASSED_SUITES * 100) / $TOTAL_SUITES" | bc)
    echo -e "Success Rate: ${YELLOW}$success_rate%${NC}"
else
    echo -e "Success Rate: ${GREEN}100%${NC}"
fi

echo ""
echo "Full report saved to: $REPORT_FILE"
echo "Test outputs saved to: $RESULTS_DIR/"
echo ""

# Update report summary
sed -i.bak "s/## Test Execution Summary/## Test Execution Summary\n\n- **Total Test Suites:** $TOTAL_SUITES\n- **Passed:** $PASSED_SUITES ✅\n- **Failed:** $FAILED_SUITES ❌\n- **Success Rate:** $(echo "scale=2; ($PASSED_SUITES * 100) / $TOTAL_SUITES" | bc)%\n\n**Detailed results below.**/" "$REPORT_FILE"
rm -f "$REPORT_FILE.bak"

# Check for load test tools
echo "" >> "$REPORT_FILE"
echo "## Additional Testing Tools" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

if command -v artillery &> /dev/null; then
    echo "- ✅ **Artillery:** Installed ($(artillery --version))" >> "$REPORT_FILE"
    echo -e "${GREEN}✓${NC} Artillery installed - load tests available"
else
    echo "- ❌ **Artillery:** Not installed" >> "$REPORT_FILE"
    echo -e "${YELLOW}⚠${NC} Artillery not installed - load tests unavailable"
    echo "  Install: npm install -g artillery"
fi

# Check for comparison test requirements
if command -v jq &> /dev/null; then
    echo "- ✅ **jq:** Installed ($(jq --version))" >> "$REPORT_FILE"
else
    echo "- ❌ **jq:** Not installed (required for comparison tests)" >> "$REPORT_FILE"
fi

if command -v bc &> /dev/null; then
    echo "- ✅ **bc:** Installed" >> "$REPORT_FILE"
else
    echo "- ❌ **bc:** Not installed (required for comparison tests)" >> "$REPORT_FILE"
fi

echo ""
echo "============================================="
echo ""
echo "Next steps:"
echo "1. Review detailed report: cat $REPORT_FILE"
echo "2. Fix failing tests identified in report"
echo "3. Run load tests: artillery run tests/load/ar-load-test.yml"
echo "4. Run comparison tests: ./tests/compare-implementations.sh"
echo "5. Run data validation: ./tests/validate-data-migration.sh"
echo ""

# Exit with error if any tests failed
if [ $FAILED_SUITES -gt 0 ]; then
    exit 1
fi

exit 0
