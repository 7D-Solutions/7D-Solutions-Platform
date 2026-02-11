#!/bin/bash

# Test concurrency limiter with MAX_CONCURRENT_HASHES=1
# Expected: 1 succeeds, others get 503 "auth busy"

echo "Hammering login endpoint with 5 parallel requests..."
echo "MAX_CONCURRENT_HASHES=1, so only 1 should succeed at a time"
echo ""

# Function to test login
test_login() {
    local id=$1
    local start=$(date +%s%N)

    response=$(curl -s -w "\n%{http_code}" -X POST http://localhost:8081/api/auth/login \
        -H "Content-Type: application/json" \
        -d '{"tenant_id":"123e4567-e89b-12d3-a456-426614174000","email":"testuser@example.com","password":"ValidPassword123"}')

    local end=$(date +%s%N)
    local duration=$(( (end - start) / 1000000 ))

    http_code=$(echo "$response" | tail -1)
    body=$(echo "$response" | head -n -1)

    if [ "$http_code" = "200" ]; then
        echo "[$id] ✅ SUCCESS (${duration}ms) - Got token"
    elif [ "$http_code" = "503" ]; then
        echo "[$id] 🔒 BUSY (${duration}ms) - $body"
    else
        echo "[$id] ❌ ERROR $http_code (${duration}ms) - $body"
    fi
}

# Launch 5 parallel requests
for i in {1..5}; do
    test_login $i &
done

# Wait for all to complete
wait

echo ""
echo "Checking metrics for hash_busy counter..."
curl -s http://localhost:8081/metrics | grep -E "auth_hash_busy|auth_login_total" | grep -v "#"
