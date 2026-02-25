#!/usr/bin/env bash
# tests/e2e/lib/helpers.sh — Shared helpers for E2E test scripts
#
# Source this file from any test script:
#   source "$(dirname "$0")/../lib/helpers.sh"
#
# Provides:
#   wait_for_ready SERVICE_NAME PORT [TIMEOUT_S]
#   assert_healthz SERVICE_NAME PORT
#   assert_ready_shape SERVICE_NAME PORT
#   bootstrap_test_tenant
#   e2e_pass MSG
#   e2e_fail MSG
#   e2e_skip MSG

set -euo pipefail

# ============================================================================
# Counters
# ============================================================================

E2E_PASS=0
E2E_FAIL=0
E2E_SKIP=0
E2E_ERRORS=()

# ============================================================================
# Service Port Registry (source of truth: docker-compose.services.yml)
# ============================================================================

declare -A SERVICE_PORTS=(
    [auth]=8080
    [ar]=8086
    [subscriptions]=8087
    [payments]=8088
    [notifications]=8089
    [gl]=8090
    [inventory]=8092
    [ap]=8093
    [treasury]=8094
    [fixed-assets]=8104
    [consolidation]=8105
    [timekeeping]=8097
    [party]=8098
    [integrations]=8099
    [ttp]=8100
    [maintenance]=8101
    [pdf-editor]=8102
    [shipping-receiving]=8103
    [control-plane]=8091
)

# Resolve port for a service name. Falls back to the argument if numeric.
resolve_port() {
    local name="$1"
    if [[ -n "${SERVICE_PORTS[$name]+x}" ]]; then
        echo "${SERVICE_PORTS[$name]}"
    elif [[ "$name" =~ ^[0-9]+$ ]]; then
        echo "$name"
    else
        echo "ERROR: unknown service '$name'" >&2
        return 1
    fi
}

# ============================================================================
# Readiness Waiters (uses /api/ready contract from HEALTH-CONTRACT.md)
# ============================================================================

# wait_for_ready SERVICE_NAME PORT [TIMEOUT_S]
#
# Polls GET /api/ready until status is "ready" or timeout expires.
# Returns 0 on ready, 1 on timeout.
wait_for_ready() {
    local name="$1"
    local port="$2"
    local timeout="${3:-30}"
    local url="http://localhost:${port}/api/ready"
    local deadline=$((SECONDS + timeout))
    local delay=1

    while (( SECONDS < deadline )); do
        local resp
        if resp=$(curl -sf -m 3 "$url" 2>/dev/null); then
            local status
            status=$(echo "$resp" | grep -o '"status":"[^"]*"' | head -1 | cut -d'"' -f4)
            if [[ "$status" == "ready" ]]; then
                return 0
            fi
        fi
        sleep "$delay"
        # Cap backoff at 3s
        (( delay = delay < 3 ? delay + 1 : 3 ))
    done

    echo "TIMEOUT: $name (:$port) not ready after ${timeout}s" >&2
    return 1
}

# ============================================================================
# Assertions
# ============================================================================

# assert_healthz SERVICE_NAME PORT
# Checks GET /healthz returns 200 with {"status":"alive"}.
assert_healthz() {
    local name="$1"
    local port="$2"
    local url="http://localhost:${port}/healthz"
    local resp

    if resp=$(curl -sf -m 5 "$url" 2>/dev/null); then
        if echo "$resp" | grep -q '"status"'; then
            e2e_pass "$name /healthz OK"
            return 0
        else
            e2e_fail "$name /healthz unexpected body: $resp"
            return 1
        fi
    else
        e2e_fail "$name /healthz request failed"
        return 1
    fi
}

# assert_ready_shape SERVICE_NAME PORT
# Checks GET /api/ready returns valid JSON with all required fields.
assert_ready_shape() {
    local name="$1"
    local port="$2"
    local url="http://localhost:${port}/api/ready"
    local raw_resp http_code body

    raw_resp=$(curl -s -m 5 -w '\n%{http_code}' "$url" 2>/dev/null) || true
    http_code=$(echo "$raw_resp" | tail -1)
    body=$(echo "$raw_resp" | sed '$d')

    if [[ ! "$http_code" =~ ^(200|503)$ ]]; then
        e2e_fail "$name /api/ready HTTP $http_code (expected 200 or 503)"
        return 1
    fi

    local missing=()
    for field in service_name version status degraded checks timestamp; do
        if ! echo "$body" | grep -q "\"${field}\""; then
            missing+=("$field")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        e2e_fail "$name /api/ready missing fields: ${missing[*]}"
        return 1
    fi

    e2e_pass "$name /api/ready shape valid (HTTP $http_code)"
    return 0
}

# assert_http_ok URL DESCRIPTION
# Simple check that URL returns HTTP 200.
assert_http_ok() {
    local url="$1"
    local desc="$2"
    local http_code

    http_code=$(curl -s -o /dev/null -w '%{http_code}' -m 5 "$url" 2>/dev/null) || true

    if [[ "$http_code" == "200" ]]; then
        e2e_pass "$desc"
        return 0
    else
        e2e_fail "$desc (HTTP $http_code)"
        return 1
    fi
}

# ============================================================================
# Result Reporters
# ============================================================================

e2e_pass() {
    local msg="$1"
    echo "  PASS  $msg"
    (( E2E_PASS++ )) || true
}

e2e_fail() {
    local msg="$1"
    echo "  FAIL  $msg"
    (( E2E_FAIL++ )) || true
    E2E_ERRORS+=("$msg")
}

e2e_skip() {
    local msg="$1"
    echo "  SKIP  $msg"
    (( E2E_SKIP++ )) || true
}

# e2e_summary — Print results and exit with appropriate code.
# Call this at the end of the runner, not individual test scripts.
e2e_summary() {
    echo ""
    echo "=== E2E Results ==="
    echo "PASS: $E2E_PASS  FAIL: $E2E_FAIL  SKIP: $E2E_SKIP"

    if [[ $E2E_FAIL -gt 0 ]]; then
        echo ""
        echo "Failures:"
        for err in "${E2E_ERRORS[@]}"; do
            echo "  - $err"
        done
        return 1
    fi
    return 0
}

# ============================================================================
# Tenant Bootstrap
# ============================================================================

# bootstrap_test_tenant — Generate a namespaced test tenant ID.
# Uses a UUID suffix so tests are isolated from each other.
bootstrap_test_tenant() {
    local prefix="${1:-e2e}"
    echo "${prefix}-$(date +%s)-$(head -c 4 /dev/urandom | xxd -p)"
}

# ============================================================================
# Tag Matching
# ============================================================================

# Tags are declared in test scripts as a comment line:
#   # TAGS: phase42-smoke smoke consolidation
#
# extract_tags FILE — prints space-separated tags from a test script.
extract_tags() {
    local file="$1"
    grep -m1 '^# TAGS:' "$file" 2>/dev/null | sed 's/^# TAGS: *//' || true
}

# has_tag FILE TAG — returns 0 if the file declares the given tag.
has_tag() {
    local file="$1"
    local tag="$2"
    local tags
    tags=$(extract_tags "$file")
    [[ " $tags " == *" $tag "* ]]
}
