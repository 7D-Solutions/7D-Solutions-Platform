#!/usr/bin/env bash
# qbo-cutover-preflight.sh
#
# Validates that all QBO production secrets and environment configuration are
# correct before executing the production cutover.
#
# Run this from the repo root before restarting the integrations service.
# Exit 0 = all checks passed. Exit 1 = one or more checks failed.
#
# Usage:
#   ./scripts/qbo-cutover-preflight.sh
#   ./scripts/qbo-cutover-preflight.sh --secrets-dir /etc/7d/production/secrets
#
# For local/sandbox environments, pass --allow-sandbox to skip production-only
# checks (e.g., when testing the script itself in a dev environment).

set -euo pipefail

SECRETS_DIR="/etc/7d/production/secrets"
ALLOW_SANDBOX=0
FAIL_COUNT=0

# ── Argument parsing ──────────────────────────────────────────────────────────

for arg in "$@"; do
  case "$arg" in
    --secrets-dir=*) SECRETS_DIR="${arg#*=}" ;;
    --secrets-dir)   shift; SECRETS_DIR="$1" ;;
    --allow-sandbox) ALLOW_SANDBOX=1 ;;
  esac
done

# ── Helpers ───────────────────────────────────────────────────────────────────

pass() { echo "  [PASS] $1"; }
fail() { echo "  [FAIL] $1"; FAIL_COUNT=$((FAIL_COUNT + 1)); }
info() { echo "  [INFO] $1"; }

secret_value() {
  local name="$1"
  # Check secrets dir first, then env var (lowercase), then env var (uppercase)
  if [ -f "$SECRETS_DIR/$name" ]; then
    cat "$SECRETS_DIR/$name"
  else
    local upper
    upper="$(echo "$name" | tr '[:lower:]' '[:upper:]')"
    printenv "$upper" 2>/dev/null || printenv "$name" 2>/dev/null || true
  fi
}

secret_exists() {
  local name="$1"
  local val
  val="$(secret_value "$name")"
  [ -n "$val" ]
}

# ── Section 1: Required secrets present ───────────────────────────────────────

echo ""
echo "=== Section 1: Required secrets ==="

for secret in qbo_client_id qbo_client_secret qbo_redirect_uri oauth_encryption_key; do
  if secret_exists "$secret"; then
    pass "$secret is set"
  else
    fail "$secret is missing or empty (check $SECRETS_DIR/$secret)"
  fi
done

# ── Section 2: Value validation ───────────────────────────────────────────────

echo ""
echo "=== Section 2: Value validation ==="

# Redirect URI must start with https:// in production
redirect_uri="$(secret_value qbo_redirect_uri)"
if [ -n "$redirect_uri" ]; then
  if echo "$redirect_uri" | grep -q "^https://"; then
    pass "QBO_REDIRECT_URI starts with https://"
  elif [ "$ALLOW_SANDBOX" = "1" ] && echo "$redirect_uri" | grep -q "^http://localhost"; then
    pass "QBO_REDIRECT_URI is localhost (allowed with --allow-sandbox)"
  else
    fail "QBO_REDIRECT_URI '$redirect_uri' does not start with https:// (required for production)"
  fi

  # Must not contain sandbox-quickbooks
  if echo "$redirect_uri" | grep -qi "sandbox"; then
    fail "QBO_REDIRECT_URI contains 'sandbox' — this is a sandbox URL, not production"
  else
    pass "QBO_REDIRECT_URI does not contain 'sandbox'"
  fi
fi

# OAUTH_ENCRYPTION_KEY should look like a hex string of reasonable length
enc_key="$(secret_value oauth_encryption_key)"
if [ -n "$enc_key" ]; then
  key_len="${#enc_key}"
  if [ "$key_len" -ge 32 ]; then
    pass "OAUTH_ENCRYPTION_KEY length is $key_len characters (>= 32)"
  else
    fail "OAUTH_ENCRYPTION_KEY is only $key_len characters — use at least 32 random hex chars"
  fi
fi

# ── Section 3: Sandbox guard ──────────────────────────────────────────────────

echo ""
echo "=== Section 3: Sandbox guard ==="

if [ "$ALLOW_SANDBOX" = "1" ]; then
  info "Sandbox checks skipped (--allow-sandbox passed)"
else
  # QBO_SANDBOX must be absent or 0
  qbo_sandbox="${QBO_SANDBOX:-}"
  if [ -z "$qbo_sandbox" ] || [ "$qbo_sandbox" = "0" ]; then
    pass "QBO_SANDBOX is absent or 0 (production mode)"
  else
    fail "QBO_SANDBOX=$qbo_sandbox — this forces CDC to use the sandbox base URL; unset it for production"
  fi

  # QBO_BASE_URL must be absent or production
  qbo_base_url="${QBO_BASE_URL:-}"
  if [ -z "$qbo_base_url" ]; then
    pass "QBO_BASE_URL is absent (service defaults to production URL)"
  elif echo "$qbo_base_url" | grep -qi "sandbox"; then
    fail "QBO_BASE_URL='$qbo_base_url' contains 'sandbox' — CDC will poll sandbox data in production"
  elif echo "$qbo_base_url" | grep -q "quickbooks.api.intuit.com"; then
    pass "QBO_BASE_URL='$qbo_base_url' is the production URL"
  else
    fail "QBO_BASE_URL='$qbo_base_url' is not a recognized Intuit API URL"
  fi

  # Check secrets dir for any sandbox references
  if grep -r "sandbox-quickbooks" "$SECRETS_DIR/" 2>/dev/null | grep -q .; then
    fail "Found 'sandbox-quickbooks' in secrets at $SECRETS_DIR — purge sandbox values before cutover"
  else
    pass "No sandbox-quickbooks references found in $SECRETS_DIR"
  fi
fi

# ── Section 4: Service connectivity ──────────────────────────────────────────

echo ""
echo "=== Section 4: Service connectivity ==="

# Check if the integrations service port is up (internal port 8099)
integrations_host="${INTEGRATIONS_HOST:-localhost}"
integrations_port="${INTEGRATIONS_PORT:-8099}"
if curl -sf --connect-timeout 3 "http://$integrations_host:$integrations_port/ops/ready" > /dev/null 2>&1; then
  pass "Integrations service is reachable at $integrations_host:$integrations_port"
else
  info "Integrations service not reachable at $integrations_host:$integrations_port (may not be started yet — non-fatal)"
fi

# ── Section 5: Intuit token endpoint reachability ─────────────────────────────

echo ""
echo "=== Section 5: Intuit endpoint reachability ==="

token_url="https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer"
# Use --max-time and check that we get any HTTP response (even 4xx means the server is up).
token_http_code=$(curl --connect-timeout 5 --max-time 8 -s -o /dev/null \
  -w "%{http_code}" -X POST "$token_url" 2>/dev/null || true)
if [ -n "$token_http_code" ] && [ "$token_http_code" -ge 100 ] 2>/dev/null; then
  pass "Intuit token endpoint reachable (HTTP $token_http_code)"
else
  fail "Cannot reach Intuit token endpoint $token_url — check network/firewall (got: '$token_http_code')"
fi

intuit_auth_url="https://appcenter.intuit.com/connect/oauth2"
if curl -sf --connect-timeout 5 -o /dev/null -I "$intuit_auth_url" > /dev/null 2>&1; then
  pass "Intuit auth endpoint reachable ($intuit_auth_url)"
else
  fail "Cannot reach Intuit auth endpoint $intuit_auth_url — check network/firewall"
fi

# ── Section 6: Database state ─────────────────────────────────────────────────

echo ""
echo "=== Section 6: Database state ==="

db_url="$(secret_value integrations_database_url)"
if [ -z "$db_url" ]; then
  db_url="${DATABASE_URL:-}"
fi

if [ -n "$db_url" ]; then
  # Check for active sandbox connections (realm IDs starting with test or well-known sandbox IDs)
  connected_count=$(psql "$db_url" -tAc \
    "SELECT COUNT(*) FROM integrations_oauth_connections WHERE provider='quickbooks' AND connection_status='connected';" \
    2>/dev/null || echo "")
  if [ -n "$connected_count" ]; then
    info "Active QBO connections: $connected_count (expected 0 before cutover; will be replaced)"
  fi

  # Check for inflight push attempts
  inflight=$(psql "$db_url" -tAc \
    "SELECT COUNT(*) FROM integrations_sync_push_attempts WHERE status='inflight';" \
    2>/dev/null || echo "")
  if [ -n "$inflight" ]; then
    if [ "$inflight" = "0" ]; then
      pass "No inflight push attempts"
    else
      fail "$inflight push attempts in 'inflight' status — drain or resolve before cutover"
    fi
  else
    info "Database not reachable for state checks (set DATABASE_URL or $SECRETS_DIR/integrations_database_url)"
  fi
else
  info "DATABASE_URL not available — skipping database state checks"
fi

# ── Section 7: Event bus / NATS ───────────────────────────────────────────────
#
# Sync events (authority changes, conflict notifications, push failures) are
# published via NATS. In-memory bus silently drops events in production.

echo ""
echo "=== Section 7: Event bus / NATS ==="

bus_type_val="$(secret_value bus_type)"
if [ -z "$bus_type_val" ]; then
  bus_type_val="${BUS_TYPE:-}"
fi
bus_type_lower="$(echo "$bus_type_val" | tr '[:upper:]' '[:lower:]')"

if [ "$ALLOW_SANDBOX" = "1" ]; then
  info "Event bus checks skipped (--allow-sandbox passed)"
else
  if [ -z "$bus_type_lower" ] || [ "$bus_type_lower" = "inmemory" ]; then
    fail "BUS_TYPE is '${bus_type_val:-unset}' — production requires BUS_TYPE=nats. \
Sync events would be silently dropped with in-memory bus."
  else
    pass "BUS_TYPE=$bus_type_val (not inmemory)"
  fi

  nats_url_val="$(secret_value nats_url)"
  if [ -z "$nats_url_val" ]; then
    nats_url_val="${NATS_URL:-}"
  fi

  if [ -z "$nats_url_val" ]; then
    fail "NATS_URL is not set — required for sync event delivery in production"
  else
    pass "NATS_URL is set"

    # Extract host:port from NATS URL (strip scheme and credentials)
    nats_host_port="$(echo "$nats_url_val" | sed 's|^nats://||; s|^[^@]*@||; s|/.*||')"
    nats_host="$(echo "$nats_host_port" | cut -d: -f1)"
    nats_port="$(echo "$nats_host_port" | cut -d: -f2)"
    nats_port="${nats_port:-4222}"

    if nc -z -w 3 "$nats_host" "$nats_port" 2>/dev/null; then
      pass "NATS server reachable at $nats_host:$nats_port"
    else
      fail "Cannot reach NATS server at $nats_host:$nats_port — check NATS_URL and network"
    fi
  fi
fi

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "========================================"
if [ "$FAIL_COUNT" -eq 0 ]; then
  echo "PREFLIGHT PASSED — $FAIL_COUNT failures"
  echo "Proceed to Phase 4: Cutover Execution"
  echo "See docs/runbooks/qbo-production-cutover.md"
  exit 0
else
  echo "PREFLIGHT FAILED — $FAIL_COUNT failure(s)"
  echo "Resolve all failures before proceeding with cutover."
  echo "See docs/runbooks/qbo-production-cutover.md for remediation steps."
  exit 1
fi
