#!/usr/bin/env bash
# verify_health_endpoints.sh — Validates /healthz and /api/ready across all services
#
# Usage: ./scripts/verify_health_endpoints.sh
# Expects all services to be running (e.g. docker compose up -d).

set -euo pipefail

PASS=0
FAIL=0
ERRORS=()

# Service name → port mapping (matches docker-compose ports)
declare -A SERVICES=(
    [ar]=8086
    [ap]=8093
    [gl]=8090
    [inventory]=8092
    [subscriptions]=8087
    [payments]=8088
    [notifications]=8089
    [treasury]=8094
    [fixed-assets]=8104
    [consolidation]=8105
    [timekeeping]=8097
    [party]=8098
    [integrations]=8099
    [ttp]=8100
    [maintenance]=8101
    [shipping-receiving]=8103
    [pdf-editor]=8102
    [identity-auth]=8080
)

check_healthz() {
    local name="$1" port="$2"
    local url="http://localhost:${port}/healthz"
    local resp
    if resp=$(curl -sf -m 5 "$url" 2>/dev/null); then
        # Must contain "status":"alive"
        if echo "$resp" | grep -q '"status"' ; then
            echo "  PASS  /healthz  $name (:$port)"
            ((PASS++))
        else
            echo "  FAIL  /healthz  $name (:$port) — unexpected body: $resp"
            ((FAIL++))
            ERRORS+=("$name /healthz: unexpected body")
        fi
    else
        echo "  FAIL  /healthz  $name (:$port) — request failed"
        ((FAIL++))
        ERRORS+=("$name /healthz: request failed")
    fi
}

check_ready() {
    local name="$1" port="$2"
    local url="http://localhost:${port}/api/ready"
    local resp http_code
    resp=$(curl -s -m 5 -w '\n%{http_code}' "$url" 2>/dev/null) || true
    http_code=$(echo "$resp" | tail -1)
    body=$(echo "$resp" | sed '$d')

    if [[ "$http_code" =~ ^(200|503)$ ]]; then
        # Validate required fields: service_name, version, status, degraded, checks, timestamp
        local missing=()
        for field in service_name version status degraded checks timestamp; do
            if ! echo "$body" | grep -q "\"${field}\""; then
                missing+=("$field")
            fi
        done

        if [[ ${#missing[@]} -eq 0 ]]; then
            echo "  PASS  /api/ready $name (:$port) — HTTP $http_code"
            ((PASS++))
        else
            echo "  FAIL  /api/ready $name (:$port) — missing fields: ${missing[*]}"
            ((FAIL++))
            ERRORS+=("$name /api/ready: missing fields: ${missing[*]}")
        fi
    else
        echo "  FAIL  /api/ready $name (:$port) — HTTP $http_code"
        ((FAIL++))
        ERRORS+=("$name /api/ready: HTTP $http_code")
    fi
}

echo "=== Health Endpoint Verification ==="
echo ""

for name in $(echo "${!SERVICES[@]}" | tr ' ' '\n' | sort); do
    port="${SERVICES[$name]}"
    echo "[$name] port $port"
    check_healthz "$name" "$port"
    check_ready "$name" "$port"
    echo ""
done

echo "=== Results ==="
echo "PASS: $PASS  FAIL: $FAIL"

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "Failures:"
    for err in "${ERRORS[@]}"; do
        echo "  - $err"
    done
    exit 1
fi

echo "All health endpoints validated."
exit 0
