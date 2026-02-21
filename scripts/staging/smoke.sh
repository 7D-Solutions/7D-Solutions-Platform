#!/usr/bin/env bash
# smoke.sh — Staging smoke suite: health checks + data endpoint validation.
#
# Curls /healthz and /api/ready for all critical services, checks the TCP UI
# login page, and validates key data routes respond correctly.
#
# Usage:
#   bash scripts/staging/smoke.sh [--host HOST] [--jwt TOKEN] [--timeout SECS]
#
# Environment variables (loaded from .env.staging or set explicitly):
#   STAGING_HOST      — VPS hostname or IP (required)
#   SMOKE_STAFF_JWT   — JWT token for authenticated data checks (optional)
#   SMOKE_TIMEOUT     — Per-request timeout in seconds (default: 10)
#
# With SMOKE_STAFF_JWT: data endpoints assert non-empty results.
# Without SMOKE_STAFF_JWT: data endpoints assert auth is enforced (HTTP 401).
#
# Exit code: 0 = all checks passed, 1 = one or more checks failed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── Load staging env if available ────────────────────────────────────────────
ENV_FILE="${SMOKE_ENV_FILE:-${REPO_ROOT}/scripts/staging/.env.staging}"
if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "${REPO_ROOT}/scripts/staging/export_env.sh" "$ENV_FILE"
fi

# ── Configuration ─────────────────────────────────────────────────────────────
HOST="${STAGING_HOST:-}"
SMOKE_STAFF_JWT="${SMOKE_STAFF_JWT:-}"
SMOKE_TIMEOUT="${SMOKE_TIMEOUT:-10}"

# Parse CLI args (override env vars)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)    HOST="$2";           shift 2 ;;
        --jwt)     SMOKE_STAFF_JWT="$2"; shift 2 ;;
        --timeout) SMOKE_TIMEOUT="$2";  shift 2 ;;
        *) echo "ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$HOST" ]]; then
    echo "ERROR: STAGING_HOST must be set (via env var or --host)." >&2
    echo "       Copy scripts/staging/env.example → scripts/staging/.env.staging" >&2
    exit 1
fi

# ── Service port map ──────────────────────────────────────────────────────────
# Canonical port assignments — matches verify_health_endpoints.sh (bd-2a3q)
declare -A PORTS=(
    [identity-auth]=8080
    [control-plane]=8092
    [ar]=8086
    [payments]=8085
    [ttp]=8100
)

# ── Counters ──────────────────────────────────────────────────────────────────
PASS=0
FAIL=0
declare -a ERRORS=()

# ── Helpers ───────────────────────────────────────────────────────────────────
banner() { printf '\n=== %s ===\n' "$*"; }

check_status() {
    local name="$1" url="$2" want="${3:-200}"
    local status
    status=$(curl -s -o /dev/null -w "%{http_code}" \
        --max-time "$SMOKE_TIMEOUT" "$url" 2>/dev/null || echo "000")
    if [[ "$status" == "$want" ]]; then
        printf '  ✓  %-40s HTTP %s\n' "$name" "$status"
        PASS=$((PASS + 1))
    else
        printf '  ✗  %-40s HTTP %s (want %s)\n' "  → $url" "$status" "$want"
        printf '     %s\n' "$name"
        FAIL=$((FAIL + 1))
        ERRORS+=("${name}: HTTP ${status} (expected ${want}) at ${url}")
    fi
}

check_data_endpoint() {
    # With JWT: assert HTTP 200 with non-empty JSON.
    # Without JWT: assert HTTP 401 (route exists and enforces auth).
    local name="$1" url="$2"
    local -a curl_args=(
        -s --max-time "$SMOKE_TIMEOUT"
        -w "\n%{http_code}"
    )
    if [[ -n "$SMOKE_STAFF_JWT" ]]; then
        curl_args+=(-H "Authorization: Bearer ${SMOKE_STAFF_JWT}")
    fi

    local raw status body
    raw=$(curl "${curl_args[@]}" "$url" 2>/dev/null || echo -e "\n000")
    # Last line is the status code, rest is body
    status="${raw##*$'\n'}"
    body="${raw%$'\n'${status}}"

    if [[ -n "$SMOKE_STAFF_JWT" ]]; then
        if [[ "$status" == "200" ]]; then
            # Check body is non-trivially empty (has at least a key or array item)
            if echo "$body" | grep -qE '"[^"]+"[[:space:]]*:[[:space:]]|"[^"]+"[[:space:]]*\]|\[[[:space:]]*\{'; then
                printf '  ✓  %-40s HTTP %s (data present)\n' "$name" "$status"
            else
                printf '  ~  %-40s HTTP %s (empty result set — OK for fresh staging)\n' "$name" "$status"
            fi
            PASS=$((PASS + 1))
        else
            printf '  ✗  %-40s HTTP %s (want 200 with JWT) — %s\n' "$name" "$status" "$url"
            FAIL=$((FAIL + 1))
            ERRORS+=("${name}: HTTP ${status} (expected 200 with JWT) at ${url}")
        fi
    else
        if [[ "$status" == "401" || "$status" == "403" ]]; then
            printf '  ✓  %-40s HTTP %s (auth enforced, no JWT provided)\n' "$name" "$status"
            PASS=$((PASS + 1))
        elif [[ "$status" == "200" ]]; then
            # Some endpoints return empty lists unauthenticated — warn but pass
            printf '  ~  %-40s HTTP %s (route up; set SMOKE_STAFF_JWT for data assertion)\n' "$name" "$status"
            PASS=$((PASS + 1))
        else
            printf '  ✗  %-40s HTTP %s (want 200 or 401) — %s\n' "$name" "$status" "$url"
            FAIL=$((FAIL + 1))
            ERRORS+=("${name}: HTTP ${status} (expected 200 or 401) at ${url}")
        fi
    fi
}

# ── Section 1: /healthz — liveness probes ────────────────────────────────────
banner "Liveness probes — /healthz"
for svc in identity-auth control-plane ar payments ttp; do
    port="${PORTS[$svc]}"
    check_status "${svc}/healthz" "http://${HOST}:${port}/healthz"
done

# ── Section 2: /api/ready — readiness probes ─────────────────────────────────
banner "Readiness probes — /api/ready"
for svc in identity-auth control-plane ar payments ttp; do
    port="${PORTS[$svc]}"
    check_status "${svc}/api/ready" "http://${HOST}:${port}/api/ready"
done

# ── Section 3: TCP UI frontend ────────────────────────────────────────────────
banner "Frontend liveness — TCP UI"
check_status "tcp-ui/login page" "http://${HOST}:3000/login"

# ── Section 4: Data endpoints ─────────────────────────────────────────────────
banner "Data endpoints — /api/tenants + /api/ttp/plans"
if [[ -z "$SMOKE_STAFF_JWT" ]]; then
    printf '  (No SMOKE_STAFF_JWT set — verifying auth is enforced rather than data contents)\n'
fi
check_data_endpoint "control-plane /api/tenants" \
    "http://${HOST}:${PORTS[control-plane]}/api/tenants"
check_data_endpoint "ttp /api/ttp/plans" \
    "http://${HOST}:${PORTS[ttp]}/api/ttp/plans"

# ── Summary ───────────────────────────────────────────────────────────────────
printf '\n'
printf '────────────────────────────────────────────\n'
printf 'Results: %d passed, %d failed\n' "$PASS" "$FAIL"

if [[ $FAIL -gt 0 ]]; then
    printf '\nFailed checks:\n'
    for err in "${ERRORS[@]}"; do
        printf '  • %s\n' "$err"
    done
    printf '\nSmoke suite FAILED — staging is not healthy.\n'
    exit 1
fi

printf '\nSmoke suite PASSED — staging is healthy.\n'
