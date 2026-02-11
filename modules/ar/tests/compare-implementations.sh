#!/bin/bash
# Compare Node.js AR implementation vs Rust AR implementation
# This script tests the same API calls against both services and compares responses

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
NODE_URL="${NODE_AR_URL:-http://localhost:3001}"
RUST_URL="${RUST_AR_URL:-http://localhost:8086}"
RESULTS_DIR="./tests/load/comparison-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/comparison-report-$TIMESTAMP.md"

# Create results directory
mkdir -p "$RESULTS_DIR"

echo "============================================="
echo "AR Implementation Comparison Test"
echo "============================================="
echo "Node.js URL: $NODE_URL"
echo "Rust URL: $RUST_URL"
echo "Report: $REPORT_FILE"
echo ""

# Track results
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Initialize report
cat > "$REPORT_FILE" << EOF
# AR Migration Comparison Report
**Generated:** $(date)
**Node.js Service:** $NODE_URL
**Rust Service:** $RUST_URL

## Summary
EOF

# Helper function to compare JSON responses
compare_responses() {
    local test_name="$1"
    local node_response="$2"
    local rust_response="$3"
    local node_status="$4"
    local rust_status="$5"

    TOTAL_TESTS=$((TOTAL_TESTS + 1))

    echo "Testing: $test_name"

    # Compare status codes
    if [ "$node_status" != "$rust_status" ]; then
        echo -e "${RED}✗ FAILED${NC}: Status code mismatch (Node: $node_status, Rust: $rust_status)"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        echo "- ❌ **$test_name**: Status code mismatch (Node: $node_status, Rust: $rust_status)" >> "$REPORT_FILE"
        return 1
    fi

    # Compare response structure (ignore timestamps and IDs that might differ)
    # Extract keys from both responses
    node_keys=$(echo "$node_response" | jq -r 'keys[]' 2>/dev/null | sort)
    rust_keys=$(echo "$rust_response" | jq -r 'keys[]' 2>/dev/null | sort)

    if [ "$node_keys" != "$rust_keys" ]; then
        echo -e "${YELLOW}⚠ WARNING${NC}: Response structure differs"
        echo "  Node keys: $node_keys"
        echo "  Rust keys: $rust_keys"
    fi

    echo -e "${GREEN}✓ PASSED${NC}: $test_name"
    PASSED_TESTS=$((PASSED_TESTS + 1))
    echo "- ✅ **$test_name**: Passed" >> "$REPORT_FILE"
    return 0
}

# Helper function to test endpoint
test_endpoint() {
    local method="$1"
    local path="$2"
    local body="$3"
    local test_name="$4"

    echo ""
    echo "-------------------------------------------"

    # Call Node.js endpoint
    if [ -n "$body" ]; then
        node_response=$(curl -s -w "\n%{http_code}" -X "$method" \
            -H "Content-Type: application/json" \
            -d "$body" \
            "$NODE_URL$path")
    else
        node_response=$(curl -s -w "\n%{http_code}" -X "$method" \
            "$NODE_URL$path")
    fi

    node_body=$(echo "$node_response" | head -n -1)
    node_status=$(echo "$node_response" | tail -n 1)

    # Call Rust endpoint
    if [ -n "$body" ]; then
        rust_response=$(curl -s -w "\n%{http_code}" -X "$method" \
            -H "Content-Type: application/json" \
            -d "$body" \
            "$RUST_URL$path")
    else
        rust_response=$(curl -s -w "\n%{http_code}" -X "$method" \
            "$RUST_URL$path")
    fi

    rust_body=$(echo "$rust_response" | head -n -1)
    rust_status=$(echo "$rust_response" | tail -n 1)

    # Compare responses
    compare_responses "$test_name" "$node_body" "$rust_body" "$node_status" "$rust_status"
}

# Helper function to measure performance
measure_performance() {
    local method="$1"
    local url="$2"
    local body="$3"
    local iterations="${4:-100}"

    local total_time=0

    for i in $(seq 1 $iterations); do
        if [ -n "$body" ]; then
            response_time=$(curl -s -w "%{time_total}" -o /dev/null -X "$method" \
                -H "Content-Type: application/json" \
                -d "$body" \
                "$url")
        else
            response_time=$(curl -s -w "%{time_total}" -o /dev/null -X "$method" "$url")
        fi

        total_time=$(echo "$total_time + $response_time" | bc)
    done

    # Calculate average (in seconds)
    avg_time=$(echo "scale=4; $total_time / $iterations" | bc)
    # Convert to milliseconds
    avg_ms=$(echo "scale=2; $avg_time * 1000" | bc)

    echo "$avg_ms"
}

echo "## Test Results" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# Test 1: Health check
test_endpoint "GET" "/api/health" "" "Health Check"

# Test 2: Create customer
CUSTOMER_EMAIL="compare-test-$(uuidgen | tr '[:upper:]' '[:lower:]')@example.com"
CUSTOMER_BODY=$(cat <<EOF
{
  "email": "$CUSTOMER_EMAIL",
  "name": "Comparison Test Customer",
  "external_customer_id": "ext-$(uuidgen | tr '[:upper:]' '[:lower:]')",
  "metadata": {"source": "comparison_test"}
}
EOF
)
test_endpoint "POST" "/api/ar/customers" "$CUSTOMER_BODY" "Create Customer"

# Test 3: List customers
test_endpoint "GET" "/api/ar/customers?limit=10" "" "List Customers"

# Test 4: List subscriptions
test_endpoint "GET" "/api/ar/subscriptions?limit=10" "" "List Subscriptions"

# Test 5: List charges
test_endpoint "GET" "/api/ar/charges?limit=10" "" "List Charges"

# Test 6: List invoices
test_endpoint "GET" "/api/ar/invoices?limit=10" "" "List Invoices"

# Test 7: List events
test_endpoint "GET" "/api/ar/events?limit=10" "" "List Events"

# Performance comparison
echo ""
echo "============================================="
echo "Performance Comparison"
echo "============================================="

echo "" >> "$REPORT_FILE"
echo "## Performance Comparison" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"
echo "| Endpoint | Node.js (ms) | Rust (ms) | Improvement |" >> "$REPORT_FILE"
echo "|----------|--------------|-----------|-------------|" >> "$REPORT_FILE"

# Test health endpoint performance
echo "Measuring health endpoint performance (100 requests)..."
node_health_time=$(measure_performance "GET" "$NODE_URL/api/health" "" 100)
rust_health_time=$(measure_performance "GET" "$RUST_URL/api/health" "" 100)
improvement=$(echo "scale=2; (($node_health_time - $rust_health_time) / $node_health_time) * 100" | bc)
echo "| Health Check | $node_health_time | $rust_health_time | ${improvement}% |" >> "$REPORT_FILE"
echo -e "Health: Node=${node_health_time}ms, Rust=${rust_health_time}ms, Improvement=${improvement}%"

# Test list customers performance
echo "Measuring list customers performance (100 requests)..."
node_list_time=$(measure_performance "GET" "$NODE_URL/api/ar/customers?limit=10" "" 100)
rust_list_time=$(measure_performance "GET" "$RUST_URL/api/ar/customers?limit=10" "" 100)
improvement=$(echo "scale=2; (($node_list_time - $rust_list_time) / $node_list_time) * 100" | bc)
echo "| List Customers | $node_list_time | $rust_list_time | ${improvement}% |" >> "$REPORT_FILE"
echo -e "List Customers: Node=${node_list_time}ms, Rust=${rust_list_time}ms, Improvement=${improvement}%"

# Final summary
echo ""
echo "============================================="
echo "Summary"
echo "============================================="
echo -e "Total Tests: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $PASSED_TESTS${NC}"
echo -e "${RED}Failed: $FAILED_TESTS${NC}"
echo ""

# Update report summary
sed -i.bak "s/## Summary/## Summary\n\n- **Total Tests:** $TOTAL_TESTS\n- **Passed:** $PASSED_TESTS ✅\n- **Failed:** $FAILED_TESTS ❌\n- **Success Rate:** $(echo "scale=2; ($PASSED_TESTS * 100) / $TOTAL_TESTS" | bc)%/" "$REPORT_FILE"
rm -f "$REPORT_FILE.bak"

echo "Full report saved to: $REPORT_FILE"

# Exit with error if any tests failed
if [ $FAILED_TESTS -gt 0 ]; then
    exit 1
fi

exit 0
