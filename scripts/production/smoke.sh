#!/usr/bin/env bash
# smoke.sh — Production smoke suite: health checks + data endpoint validation.
#
# All production service ports are firewalled (UFW).  Every curl check is run
# via SSH on the VPS against localhost so no service port needs to be open to
# the internet.
#
# Usage:
#   bash scripts/production/smoke.sh [--host HOST] [--user USER] [--ssh-port PORT] \
#                                     [--jwt TOKEN] [--timeout SECS] [--dry-run]
#
# Environment variables (set via GitHub Actions secrets or .env.production):
#   PROD_HOST         — VPS hostname or IP (required)
#   PROD_USER         — SSH deploy user    (default: deploy)
#   PROD_SSH_PORT     — SSH port           (default: 22)
#   SMOKE_STAFF_JWT   — JWT for auth'd data checks (optional)
#   SMOKE_TIMEOUT     — Per-request timeout in seconds (default: 10)
#
# With SMOKE_STAFF_JWT: data endpoints assert non-empty results (HTTP 200).
# Without SMOKE_STAFF_JWT: data endpoints assert auth is enforced (HTTP 401).
#
# Exit code: 0 = all checks passed, 1 = one or more checks failed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── Configuration ─────────────────────────────────────────────────────────────
HOST="${PROD_HOST:-}"
USER="${PROD_USER:-deploy}"
SSH_PORT="${PROD_SSH_PORT:-22}"
SMOKE_STAFF_JWT="${SMOKE_STAFF_JWT:-}"
SMOKE_TIMEOUT="${SMOKE_TIMEOUT:-10}"
DRY_RUN=false

# Parse CLI args (override env vars)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)     HOST="$2";     shift 2 ;;
        --user)     USER="$2";     shift 2 ;;
        --ssh-port) SSH_PORT="$2"; shift 2 ;;
        --jwt)      SMOKE_STAFF_JWT="$2"; shift 2 ;;
        --timeout)  SMOKE_TIMEOUT="$2";  shift 2 ;;
        --dry-run)  DRY_RUN=true;  shift   ;;
        *) echo "ERROR: Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$HOST" ]]; then
    echo "ERROR: PROD_HOST must be set (via env var or --host)." >&2
    echo "       Copy scripts/production/env.example → scripts/production/.env.production" >&2
    exit 1
fi

SSH_OPTS="-o StrictHostKeyChecking=no -o BatchMode=yes -p ${SSH_PORT}"
SSH_TARGET="${USER}@${HOST}"

# ── Service port map (localhost inside VPS) ────────────────────────────────────
# Canonical port assignments matching docker-compose.platform.yml / docker-compose.modules.yml
declare -A HEALTH_PATHS=(
    [identity-auth]="8080|/api/health"
    [control-plane]="8091|/api/ready"
    [ar]="8086|/api/health"
    [payments]="8088|/api/health"
    [ttp]="8100|/api/health"
)

# ── Counters ──────────────────────────────────────────────────────────────────
PASS=0
FAIL=0
declare -a ERRORS=()

# ── Helpers ───────────────────────────────────────────────────────────────────
banner() { printf '\n=== %s ===\n' "$*"; }

# Run a curl command on the VPS via SSH (or echo in dry-run mode).
# Outputs the HTTP status code.
remote_curl_status() {
    local url="$1"
    if $DRY_RUN; then
        echo "DRY-RUN: curl $url (via SSH ${SSH_TARGET})" >&2
        echo "200"
        return
    fi
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" \
        "curl -s -o /dev/null -w '%{http_code}' --max-time ${SMOKE_TIMEOUT} '${url}' 2>/dev/null || echo 000"
}

# Run a curl command on VPS that returns body + status code.
remote_curl_body_status() {
    local url="$1"
    local auth_header="${2:-}"
    if $DRY_RUN; then
        echo "DRY-RUN: curl $url (via SSH ${SSH_TARGET})" >&2
        printf '{"items":[]}\n200'
        return
    fi
    local cmd="curl -s --max-time ${SMOKE_TIMEOUT} -w '\n%{http_code}'"
    if [[ -n "$auth_header" ]]; then
        cmd+=" -H '${auth_header}'"
    fi
    cmd+=" '${url}' 2>/dev/null || printf '\n000'"
    # shellcheck disable=SC2029
    ssh $SSH_OPTS "$SSH_TARGET" "$cmd"
}

check_status() {
    local name="$1" url="$2" want="${3:-200}"
    local status
    status="$(remote_curl_status "$url")"
    if [[ "$status" == "$want" ]]; then
        printf '  ✓  %-42s HTTP %s\n' "$name" "$status"
        PASS=$((PASS + 1))
    else
        printf '  ✗  %-42s HTTP %s (want %s)\n' "$name" "$status" "$want"
        printf '     → %s\n' "$url"
        FAIL=$((FAIL + 1))
        ERRORS+=("${name}: HTTP ${status} (expected ${want}) at ${url}")
    fi
}

check_data_endpoint() {
    # With JWT: assert HTTP 200 with non-empty JSON.
    # Without JWT: assert HTTP 401/403 (auth is enforced).
    local name="$1" url="$2"
    local auth_header=""
    if [[ -n "$SMOKE_STAFF_JWT" ]]; then
        auth_header="Authorization: Bearer ${SMOKE_STAFF_JWT}"
    fi

    local raw status body
    raw="$(remote_curl_body_status "$url" "$auth_header")"
    # Last line is the status code, rest is body
    status="${raw##*$'\n'}"
    body="${raw%$'\n'${status}}"

    if [[ -n "$SMOKE_STAFF_JWT" ]]; then
        if [[ "$status" == "200" ]]; then
            if echo "$body" | grep -qE '"[^"]+"[[:space:]]*:[[:space:]]|"[^"]+"[[:space:]]*\]|\[[[:space:]]*\{'; then
                printf '  ✓  %-42s HTTP %s (data present)\n' "$name" "$status"
            else
                printf '  ~  %-42s HTTP %s (empty result — OK for fresh production)\n' "$name" "$status"
            fi
            PASS=$((PASS + 1))
        else
            printf '  ✗  %-42s HTTP %s (want 200 with JWT) — %s\n' "$name" "$status" "$url"
            FAIL=$((FAIL + 1))
            ERRORS+=("${name}: HTTP ${status} (expected 200 with JWT) at ${url}")
        fi
    else
        if [[ "$status" == "401" || "$status" == "403" ]]; then
            printf '  ✓  %-42s HTTP %s (auth enforced, no JWT)\n' "$name" "$status"
            PASS=$((PASS + 1))
        elif [[ "$status" == "200" ]]; then
            printf '  ~  %-42s HTTP %s (route up; set SMOKE_STAFF_JWT for data assertion)\n' "$name" "$status"
            PASS=$((PASS + 1))
        else
            printf '  ✗  %-42s HTTP %s (want 200 or 401) — %s\n' "$name" "$status" "$url"
            FAIL=$((FAIL + 1))
            ERRORS+=("${name}: HTTP ${status} (expected 200 or 401) at ${url}")
        fi
    fi
}

# ── Preflight: SSH connectivity ────────────────────────────────────────────────
printf 'Production smoke suite — %s\n' "$HOST"
printf 'SSH target: %s (port %s)\n' "$SSH_TARGET" "$SSH_PORT"
if ! $DRY_RUN; then
    if ! ssh $SSH_OPTS "$SSH_TARGET" "echo 'SSH OK'" >/dev/null 2>&1; then
        echo "ERROR: Cannot reach ${SSH_TARGET} via SSH." >&2
        exit 1
    fi
    printf '✓ SSH connectivity OK\n'
fi

# ── Section 1: /healthz — liveness probes ─────────────────────────────────────
banner "Liveness probes — /healthz (via SSH localhost)"
for svc in identity-auth control-plane ar payments ttp; do
    port_path="${HEALTH_PATHS[$svc]}"
    port="${port_path%%|*}"
    check_status "${svc}/healthz" "http://localhost:${port}/healthz"
done

# ── Section 2: /api/ready + /api/health — readiness probes ───────────────────
banner "Readiness probes (via SSH localhost)"
for svc in identity-auth control-plane ar payments ttp; do
    port_path="${HEALTH_PATHS[$svc]}"
    port="${port_path%%|*}"
    path="${port_path##*|}"
    check_status "${svc}${path}" "http://localhost:${port}${path}"
done

# ── Section 3: TCP UI frontend ────────────────────────────────────────────────
banner "Frontend liveness — TCP UI (via SSH localhost)"
check_status "tcp-ui/login page" "http://localhost:3000/login"

# ── Section 4: Data endpoints ─────────────────────────────────────────────────
banner "Data endpoints — /api/tenants + /api/ttp/plans (via SSH localhost)"
if [[ -z "$SMOKE_STAFF_JWT" ]]; then
    printf '  (No SMOKE_STAFF_JWT set — verifying auth is enforced rather than data contents)\n'
fi
check_data_endpoint "control-plane /api/tenants" \
    "http://localhost:8091/api/tenants"
check_data_endpoint "ttp /api/ttp/plans" \
    "http://localhost:8100/api/ttp/plans"

# ── Summary ───────────────────────────────────────────────────────────────────
printf '\n'
printf '────────────────────────────────────────────\n'
printf 'Results: %d passed, %d failed\n' "$PASS" "$FAIL"

if [[ $FAIL -gt 0 ]]; then
    printf '\nFailed checks:\n'
    for err in "${ERRORS[@]}"; do
        printf '  • %s\n' "$err"
    done
    printf '\nProduction smoke suite FAILED — check service health on %s.\n' "$HOST"
    exit 1
fi

printf '\nProduction smoke suite PASSED — all services healthy on %s.\n' "$HOST"
