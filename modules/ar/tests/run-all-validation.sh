#!/bin/bash
# Master validation script for AR migration
# Runs all validation tests and generates comprehensive report

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Configuration
RESULTS_DIR="./tests/load/validation-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
MASTER_REPORT="$RESULTS_DIR/master-validation-report-$TIMESTAMP.md"

# Create results directory
mkdir -p "$RESULTS_DIR"

# Track overall status
TOTAL_STAGES=6
PASSED_STAGES=0
FAILED_STAGES=0

echo ""
echo "============================================="
echo "  AR MIGRATION VALIDATION - MASTER SUITE"
echo "============================================="
echo "Timestamp: $(date)"
echo "Results: $MASTER_REPORT"
echo ""

# Initialize master report
cat > "$MASTER_REPORT" << EOF
# AR Migration - Master Validation Report

**Generated:** $(date)
**Validation Suite Version:** 1.0.0

---

## Executive Summary

This report contains results from comprehensive end-to-end validation of the AR service migration from Node.js/MySQL to Rust/PostgreSQL.

## Validation Stages

EOF

# Helper to run stage and track results
run_stage() {
    local stage_num="$1"
    local stage_name="$2"
    local command="$3"
    local skip="${4:-false}"

    echo ""
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}Stage $stage_num: $stage_name${NC}"
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    if [ "$skip" = "true" ]; then
        echo -e "${YELLOW}⊘ SKIPPED${NC}: $stage_name"
        echo "### Stage $stage_num: $stage_name - ⊘ SKIPPED" >> "$MASTER_REPORT"
        echo "" >> "$MASTER_REPORT"
        return 0
    fi

    # Run the command
    if eval "$command"; then
        echo -e "${GREEN}✓ PASSED${NC}: $stage_name"
        PASSED_STAGES=$((PASSED_STAGES + 1))
        echo "### Stage $stage_num: $stage_name - ✅ PASSED" >> "$MASTER_REPORT"
    else
        echo -e "${RED}✗ FAILED${NC}: $stage_name"
        FAILED_STAGES=$((FAILED_STAGES + 1))
        echo "### Stage $stage_num: $stage_name - ❌ FAILED" >> "$MASTER_REPORT"
    fi

    echo "" >> "$MASTER_REPORT"
}

# STAGE 1: Unit Tests
run_stage 1 "Unit Tests" \
    "cargo test --lib --quiet 2>&1 | tee $RESULTS_DIR/unit-tests-$TIMESTAMP.log"

# STAGE 2: Integration Tests
run_stage 2 "Integration Tests" \
    "cargo test --test customer_tests --test subscription_tests --test payment_tests --test webhook_tests --test idempotency_test --quiet 2>&1 | tee $RESULTS_DIR/integration-tests-$TIMESTAMP.log"

# STAGE 3: E2E Workflow Tests
echo ""
echo -e "${YELLOW}Note: E2E tests may have partial failures due to incomplete endpoint implementations${NC}"
run_stage 3 "End-to-End Workflow Tests" \
    "cargo test --test e2e_workflows --quiet 2>&1 | tee $RESULTS_DIR/e2e-tests-$TIMESTAMP.log" \
    "false"

# STAGE 4: Data Migration Validation
echo ""
echo -e "${YELLOW}Note: Data validation requires both MySQL and PostgreSQL databases to be accessible${NC}"
run_stage 4 "Data Migration Validation" \
    "./tests/validate-data-migration.sh 2>&1 | tee $RESULTS_DIR/data-validation-$TIMESTAMP.log" \
    "true"  # Skip by default - requires manual setup

# STAGE 5: Implementation Comparison
echo ""
echo -e "${YELLOW}Note: Comparison testing requires both Node.js and Rust services running${NC}"
run_stage 5 "Node.js vs Rust Comparison" \
    "./tests/compare-implementations.sh 2>&1 | tee $RESULTS_DIR/comparison-$TIMESTAMP.log" \
    "true"  # Skip by default - requires both services running

# STAGE 6: Load Testing
echo ""
echo -e "${YELLOW}Note: Load testing requires Artillery installed and Rust service running${NC}"
run_stage 6 "Load Testing" \
    "command -v artillery >/dev/null 2>&1 && artillery run tests/load/ar-load-test.yml 2>&1 | tee $RESULTS_DIR/load-test-$TIMESTAMP.log" \
    "true"  # Skip by default - requires setup

# Generate summary
echo ""
echo "============================================="
echo "  VALIDATION SUMMARY"
echo "============================================="
echo -e "Total Stages: $TOTAL_STAGES"
echo -e "${GREEN}Passed: $PASSED_STAGES${NC}"
echo -e "${RED}Failed: $FAILED_STAGES${NC}"
echo -e "Skipped: $((TOTAL_STAGES - PASSED_STAGES - FAILED_STAGES))"
echo ""

success_rate=0
if [ $((PASSED_STAGES + FAILED_STAGES)) -gt 0 ]; then
    success_rate=$(echo "scale=1; ($PASSED_STAGES * 100) / ($PASSED_STAGES + $FAILED_STAGES)" | bc)
fi

# Add summary to report
cat >> "$MASTER_REPORT" << EOF

---

## Overall Results

- **Total Validation Stages:** $TOTAL_STAGES
- **Passed:** $PASSED_STAGES ✅
- **Failed:** $FAILED_STAGES ❌
- **Skipped:** $((TOTAL_STAGES - PASSED_STAGES - FAILED_STAGES)) ⊘
- **Success Rate:** ${success_rate}%

## Detailed Logs

Individual test logs are available in:
\`$RESULTS_DIR/\`

- Unit tests: \`unit-tests-$TIMESTAMP.log\`
- Integration tests: \`integration-tests-$TIMESTAMP.log\`
- E2E tests: \`e2e-tests-$TIMESTAMP.log\`
- Data validation: \`data-validation-$TIMESTAMP.log\`
- Comparison: \`comparison-$TIMESTAMP.log\`
- Load testing: \`load-test-$TIMESTAMP.log\`

## Recommendations

EOF

# Add recommendations based on results
if [ $FAILED_STAGES -eq 0 ]; then
    echo "✅ **Migration Ready**: All automated validation stages passed." >> "$MASTER_REPORT"
    echo "" >> "$MASTER_REPORT"
    echo "Recommended next steps:" >> "$MASTER_REPORT"
    echo "1. Run manual data validation (\`./tests/validate-data-migration.sh\`)" >> "$MASTER_REPORT"
    echo "2. Run comparison testing with both services (\`./tests/compare-implementations.sh\`)" >> "$MASTER_REPORT"
    echo "3. Execute load testing (\`artillery run tests/load/ar-load-test.yml\`)" >> "$MASTER_REPORT"
    echo "4. Monitor production deployment for 24-48 hours" >> "$MASTER_REPORT"
else
    echo "⚠️  **Action Required**: Some validation stages failed." >> "$MASTER_REPORT"
    echo "" >> "$MASTER_REPORT"
    echo "Before proceeding:" >> "$MASTER_REPORT"
    echo "1. Review failed test logs in \`$RESULTS_DIR/\`" >> "$MASTER_REPORT"
    echo "2. Fix failing tests or endpoint implementations" >> "$MASTER_REPORT"
    echo "3. Re-run this validation suite" >> "$MASTER_REPORT"
    echo "4. Ensure 100% test pass rate before production deployment" >> "$MASTER_REPORT"
fi

echo "" >> "$MASTER_REPORT"
echo "---" >> "$MASTER_REPORT"
echo "*Report generated by AR Migration Validation Suite*" >> "$MASTER_REPORT"

echo ""
echo "Master report saved to:"
echo "  $MASTER_REPORT"
echo ""

# Exit with appropriate code
if [ $FAILED_STAGES -gt 0 ]; then
    exit 1
fi

exit 0
