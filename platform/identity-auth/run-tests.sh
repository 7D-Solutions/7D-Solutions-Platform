#!/bin/bash
set -e

echo "🔐 auth-rs v1.4 Test Suite"
echo "=========================="
echo ""

BASE_URL="http://localhost:8081"
TENANT_ID=$(uuidgen)
USER_ID=$(uuidgen)
EMAIL="test@example.com"
PASSWORD="TestPassword123!"

echo "📊 Test Configuration:"
echo "  Tenant ID: $TENANT_ID"
echo "  User ID: $USER_ID"
echo "  Email: $EMAIL"
echo ""

# Test 1: Health checks
echo "1️⃣  Testing /health/live..."
if curl -s -f "$BASE_URL/health/live" > /dev/null; then
    echo "   ✅ Live check passed"
else
    echo "   ❌ Live check failed"
    exit 1
fi

echo "2️⃣  Testing /health/ready..."
READY_RESPONSE=$(curl -s "$BASE_URL/health/ready")
if echo "$READY_RESPONSE" | grep -q "ready"; then
    echo "   ✅ Ready check passed"
    echo "   📝 Response: $READY_RESPONSE"
else
    echo "   ❌ Ready check failed"
    echo "   📝 Response: $READY_RESPONSE"
    exit 1
fi
echo ""

# Test 2: Register
echo "3️⃣  Testing user registration..."
REGISTER_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/auth/register" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"user_id\":\"$USER_ID\",\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")

HTTP_CODE=$(echo "$REGISTER_RESPONSE" | tail -n1)
REGISTER_BODY=$(echo "$REGISTER_RESPONSE" | head -n-1)

if [ "$HTTP_CODE" = "200" ]; then
    echo "   ✅ Registration successful"
    echo "   📝 Response: $REGISTER_BODY"
else
    echo "   ❌ Registration failed (HTTP $HTTP_CODE)"
    echo "   📝 Response: $REGISTER_BODY"
    exit 1
fi
echo ""

# Test 3: Login
echo "4️⃣  Testing user login..."
LOGIN_RESPONSE=$(curl -s -X POST "$BASE_URL/api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")

echo "$LOGIN_RESPONSE" | jq . > /dev/null 2>&1
if [ $? -eq 0 ]; then
    ACCESS_TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.access_token')
    REFRESH_TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.refresh_token')

    if [ "$ACCESS_TOKEN" != "null" ] && [ "$REFRESH_TOKEN" != "null" ]; then
        echo "   ✅ Login successful"
        echo "   🔑 Access token: ${ACCESS_TOKEN:0:30}..."
        echo "   🔄 Refresh token: ${REFRESH_TOKEN:0:30}..."
    else
        echo "   ❌ Login failed - missing tokens"
        exit 1
    fi
else
    echo "   ❌ Login failed - invalid JSON response"
    echo "   📝 Response: $LOGIN_RESPONSE"
    exit 1
fi
echo ""

# Test 4: Refresh
echo "5️⃣  Testing token refresh..."
REFRESH_RESPONSE=$(curl -s -X POST "$BASE_URL/api/auth/refresh" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}")

NEW_ACCESS=$(echo "$REFRESH_RESPONSE" | jq -r '.access_token')
NEW_REFRESH=$(echo "$REFRESH_RESPONSE" | jq -r '.refresh_token')

if [ "$NEW_ACCESS" != "null" ] && [ "$NEW_REFRESH" != "null" ]; then
    echo "   ✅ Token refresh successful"
    echo "   🔑 New access token: ${NEW_ACCESS:0:30}..."
    echo "   🔄 New refresh token: ${NEW_REFRESH:0:30}..."
    REFRESH_TOKEN=$NEW_REFRESH
else
    echo "   ❌ Token refresh failed"
    echo "   📝 Response: $REFRESH_RESPONSE"
    exit 1
fi
echo ""

# Test 5: Logout
echo "6️⃣  Testing logout..."
LOGOUT_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/auth/logout" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}")

HTTP_CODE=$(echo "$LOGOUT_RESPONSE" | tail -n1)
LOGOUT_BODY=$(echo "$LOGOUT_RESPONSE" | head -n-1)

if [ "$HTTP_CODE" = "200" ]; then
    echo "   ✅ Logout successful"
    echo "   📝 Response: $LOGOUT_BODY"
else
    echo "   ❌ Logout failed (HTTP $HTTP_CODE)"
    echo "   📝 Response: $LOGOUT_BODY"
    exit 1
fi
echo ""

# Test 6: Verify refresh token is revoked
echo "7️⃣  Testing revoked token (should fail)..."
REVOKED_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/auth/refresh" \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}")

HTTP_CODE=$(echo "$REVOKED_RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "401" ]; then
    echo "   ✅ Revoked token correctly rejected"
else
    echo "   ❌ Revoked token was accepted (security issue!)"
    exit 1
fi
echo ""

echo "=========================================="
echo "✅ All tests passed!"
echo "=========================================="
echo ""
echo "📋 Test Summary:"
echo "  ✅ Health checks (live + ready)"
echo "  ✅ User registration"
echo "  ✅ User login with JWT"
echo "  ✅ Token refresh + rotation"
echo "  ✅ User logout"
echo "  ✅ Revoked token rejection"
echo ""
echo "🎉 auth-rs v1.4 is fully operational!"
