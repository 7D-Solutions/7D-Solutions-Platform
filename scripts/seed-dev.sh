#!/usr/bin/env bash
# ============================================================
# seed-dev.sh
# Idempotent full dev environment seeder.
# Safe to run multiple times — each step skips completed work.
#
# Steps:
#   1. Preflight: verify required containers/services are up
#   2. Seed platform admin (admin@7dsolutions.local)
#   3. Provision dev tenant (dev-test-01, plan=monthly)
#   4. Seed GL accounting period for current month
#   5. Seed AR demo data (seed=42, 3 customers, 5 invoices each)
#   6. Print summary with connection hints
#
# Usage:
#   ./scripts/seed-dev.sh
#
# Environment overrides:
#   DEV_ADMIN_PASSWORD   Admin password       (default: DevAdmin1!)
#   AUTH_URL             Auth base URL        (default: http://localhost:8080)
#   AR_URL               AR module base URL   (default: http://localhost:8086)
#   AUTH_DB_CONTAINER    Auth postgres name   (default: 7d-auth-postgres)
#   TR_DB_CONTAINER      Registry postgres    (default: 7d-tenant-registry-postgres)
#   GL_DB_CONTAINER      GL postgres name     (default: 7d-gl-postgres)
# ============================================================
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── Configuration ─────────────────────────────────────────
ADMIN_EMAIL="admin@7dsolutions.local"
ADMIN_PASSWORD="${DEV_ADMIN_PASSWORD:-DevAdmin1!}"
DEV_TENANT_SLUG="dev-test-01"
AUTH_URL="${AUTH_URL:-http://localhost:8080}"
AR_URL="${AR_URL:-http://localhost:8086}"

# Deterministic v5 UUID for the tenant slug — mirrors tenantctl's parse_tenant_id logic:
#   uuid::Uuid::new_v5(&Uuid::NAMESPACE_DNS, slug.as_bytes())
DEV_TENANT_UUID=$(python3 -c "
import uuid
print(uuid.uuid5(uuid.NAMESPACE_DNS, '${DEV_TENANT_SLUG}'))
")

# DB containers + credentials (match docker-compose.data.yml defaults)
AUTH_DB_CONTAINER="${AUTH_DB_CONTAINER:-7d-auth-postgres}"
AUTH_DB_NAME="${AUTH_POSTGRES_DB:-auth_db}"
AUTH_DB_USER="${AUTH_POSTGRES_USER:-auth_user}"

TR_DB_CONTAINER="${TR_DB_CONTAINER:-7d-tenant-registry-postgres}"
TR_DB_NAME="${TENANT_REGISTRY_POSTGRES_DB:-tenant_registry_db}"
TR_DB_USER="${TENANT_REGISTRY_POSTGRES_USER:-tenant_registry_user}"

GL_DB_CONTAINER="${GL_DB_CONTAINER:-7d-gl-postgres}"
GL_DB_USER="${GL_POSTGRES_USER:-gl_user}"
# Tenant-specific GL DB name (mirrors provision.rs: "tenant_{id}_gl_db")
GL_TENANT_DB="tenant_${DEV_TENANT_UUID}_gl_db"

# ── Helpers ───────────────────────────────────────────────

# Run SQL via docker exec (stdin = heredoc or -c flag)
run_sql() {
  local container="$1" user="$2" db="$3"
  shift 3
  docker exec -i "$container" psql -U "$user" -d "$db" \
    -v ON_ERROR_STOP=1 "$@"
}

# Run SQL, return single trimmed value (no header, no trailing whitespace)
run_sql_quiet() {
  local container="$1" user="$2" db="$3"
  shift 3
  docker exec -i "$container" psql -U "$user" -d "$db" \
    -v ON_ERROR_STOP=1 -tAq "$@"
}

die() {
  echo "ERROR: $*" >&2
  exit 1
}

# ── Step 1: Preflight ─────────────────────────────────────

step1_preflight() {
  echo "=== Step 1/5: Preflight ==="
  local failed=0

  # Database containers — verify pg_isready
  local db_checks=(
    "${AUTH_DB_CONTAINER}:${AUTH_DB_USER}:${AUTH_DB_NAME}"
    "${TR_DB_CONTAINER}:${TR_DB_USER}:${TR_DB_NAME}"
    "${GL_DB_CONTAINER}:${GL_DB_USER}:gl_db"
  )
  for entry in "${db_checks[@]}"; do
    local ctr usr db rest
    ctr="${entry%%:*}"; rest="${entry#*:}"; usr="${rest%%:*}"; db="${rest#*:}"
    if docker exec "$ctr" pg_isready -U "$usr" -d "$db" >/dev/null 2>&1; then
      echo "  ✓ $ctr ready"
    else
      echo "  ✗ $ctr not ready" >&2
      failed=1
    fi
  done

  # NATS — just check the container is running
  if docker inspect --format '{{.State.Running}}' 7d-nats 2>/dev/null | grep -q true; then
    echo "  ✓ 7d-nats running"
  else
    echo "  ✗ 7d-nats not running" >&2
    failed=1
  fi

  # HTTP services — curl health endpoints
  local http_checks=(
    "auth:${AUTH_URL}/api/health"
    "ar:${AR_URL}/api/health"
    "gl:http://localhost:8090/api/health"
    "payments:http://localhost:8088/api/health"
    "subscriptions:http://localhost:8087/api/health"
  )
  for entry in "${http_checks[@]}"; do
    local svc url
    svc="${entry%%:*}"; url="${entry#*:}"
    if curl -sf --max-time 5 "$url" >/dev/null 2>&1; then
      echo "  ✓ $svc healthy ($url)"
    else
      echo "  ✗ $svc not reachable at $url" >&2
      failed=1
    fi
  done

  if [[ "$failed" -ne 0 ]]; then
    echo "" >&2
    echo "  Fix: docker compose -f docker-compose.data.yml up -d && docker compose up -d" >&2
    exit 1
  fi
  echo ""
}

# ── Step 2: Platform admin ────────────────────────────────

step2_platform_admin() {
  echo "=== Step 2/5: Seed platform admin ==="
  "$PROJECT_ROOT/scripts/seed-platform-admin.sh" \
    --email "$ADMIN_EMAIL" \
    --password "$ADMIN_PASSWORD"
  echo ""
}

# ── Step 3: Dev tenant ────────────────────────────────────

step3_provision_tenant() {
  echo "=== Step 3/5: Provision dev tenant ($DEV_TENANT_SLUG) ==="
  echo "  UUID: $DEV_TENANT_UUID"

  # Check current status in tenant registry
  local current_status
  current_status=$(run_sql_quiet "$TR_DB_CONTAINER" "$TR_DB_USER" "$TR_DB_NAME" \
    -c "SELECT status FROM tenants WHERE tenant_id = '${DEV_TENANT_UUID}'" \
    2>/dev/null || echo "")

  if [[ "$current_status" == "active" ]]; then
    echo "  ✓ Tenant already active — skipping provisioning"
  else
    if [[ -z "$current_status" ]]; then
      echo "  → Tenant not found — provisioning module databases..."
    else
      echo "  → Tenant status='$current_status' — re-provisioning..."
    fi

    # Create per-module databases and run migrations (idempotent: skips existing DBs)
    "$PROJECT_ROOT/scripts/cargo-slot.sh" run -p tenantctl -- \
      tenant create --tenant "$DEV_TENANT_SLUG"

    # Register/update tenant in registry with active status and monthly plan
    run_sql "$TR_DB_CONTAINER" "$TR_DB_USER" "$TR_DB_NAME" -q <<SQL
INSERT INTO tenants (tenant_id, status, environment, plan_code, created_at, updated_at)
VALUES ('${DEV_TENANT_UUID}', 'active', 'development', 'monthly', NOW(), NOW())
ON CONFLICT (tenant_id) DO UPDATE SET
    status     = 'active',
    plan_code  = 'monthly',
    updated_at = NOW();
SQL

    echo "  ✓ Tenant provisioned and registered"
  fi

  # Idempotent: ensure plan_code=monthly even if tenant was pre-existing
  run_sql "$TR_DB_CONTAINER" "$TR_DB_USER" "$TR_DB_NAME" -q \
    -c "UPDATE tenants
        SET plan_code = 'monthly', updated_at = NOW()
        WHERE tenant_id = '${DEV_TENANT_UUID}'
          AND (plan_code IS NULL OR plan_code <> 'monthly');"

  # Verify final state
  local final_status
  final_status=$(run_sql_quiet "$TR_DB_CONTAINER" "$TR_DB_USER" "$TR_DB_NAME" \
    -c "SELECT status FROM tenants WHERE tenant_id = '${DEV_TENANT_UUID}'" \
    2>/dev/null || echo "")
  [[ "$final_status" == "active" ]] \
    || die "Tenant status='$final_status' after provisioning (expected 'active')"

  echo "  ✓ Tenant: status=active, plan_code=monthly"
  echo ""
}

# ── Step 4: GL accounting period ──────────────────────────

step4_gl_period() {
  echo "=== Step 4/5: GL accounting period ==="

  # Verify tenant GL database was created by step 3
  local db_exists
  db_exists=$(docker exec "$GL_DB_CONTAINER" \
    psql -U "$GL_DB_USER" -d postgres -tAq \
    -c "SELECT 1 FROM pg_database WHERE datname = '${GL_TENANT_DB}'" \
    2>/dev/null || echo "")

  if [[ -z "$db_exists" ]]; then
    echo "  ⚠ GL tenant database '$GL_TENANT_DB' does not exist"
    echo "    (Step 3 should have created it — check tenantctl output above)"
    echo ""
    return 0
  fi

  # Insert current-month period if it doesn't exist.
  # Uses SELECT WHERE NOT EXISTS instead of ON CONFLICT because the EXCLUDE
  # constraint on accounting_periods is not a UNIQUE constraint.
  local inserted
  inserted=$(run_sql_quiet "$GL_DB_CONTAINER" "$GL_DB_USER" "$GL_TENANT_DB" -c "
WITH period AS (
  SELECT
    date_trunc('month', CURRENT_DATE)::date                                            AS ps,
    (date_trunc('month', CURRENT_DATE) + INTERVAL '1 month' - INTERVAL '1 day')::date AS pe
)
INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
SELECT '${DEV_TENANT_UUID}', ps, pe, false
FROM period
WHERE NOT EXISTS (
  SELECT 1 FROM accounting_periods ap
  JOIN period p ON ap.period_start = p.ps AND ap.period_end = p.pe
  WHERE ap.tenant_id = '${DEV_TENANT_UUID}'
)
RETURNING id::text" 2>/dev/null || echo "")

  if [[ -n "$inserted" ]]; then
    echo "  ✓ GL accounting period created (id=$inserted)"
  else
    echo "  ✓ GL accounting period already exists for current month"
  fi
  echo ""
}

# ── Step 5: AR demo data ──────────────────────────────────

step5_ar_demo() {
  echo "=== Step 5/5: AR demo data (seed=42, 3 customers × 5 invoices) ==="
  echo "  Idempotency keys prevent duplicates on re-runs."

  # Cargo output goes to stderr; the binary prints the dataset digest to stdout
  local digest
  digest=$("$PROJECT_ROOT/scripts/cargo-slot.sh" run -p demo-seed -- \
    --tenant "$DEV_TENANT_UUID" \
    --seed 42 \
    --customers 3 \
    --invoices-per-customer 5 \
    --ar-url "$AR_URL" 2>/dev/null)

  echo "  ✓ AR demo data seeded (dataset digest: ${digest:-<none>})"
  echo ""
}

# ── Summary ───────────────────────────────────────────────

print_summary() {
  echo "============================================================"
  echo "  Dev environment ready"
  echo "============================================================"
  echo ""
  echo "  Platform admin"
  echo "    Email:      $ADMIN_EMAIL"
  echo "    Password:   \${DEV_ADMIN_PASSWORD:-DevAdmin1!}"
  echo "    TCP UI:     http://localhost:3000"
  echo ""
  echo "  Dev tenant"
  echo "    Slug:       $DEV_TENANT_SLUG"
  echo "    UUID:       $DEV_TENANT_UUID"
  echo "    Status:     active"
  echo "    Plan:       monthly"
  echo ""
  echo "  AR demo data (seed=42, idempotent)"
  echo "    Customers:  3"
  echo "    Invoices:   15 (5 per customer)"
  echo ""
  echo "  DB access — docker exec"
  printf "    Auth:       docker exec -it %s psql -U %s -d %s\n" \
    "$AUTH_DB_CONTAINER" "$AUTH_DB_USER" "$AUTH_DB_NAME"
  printf "    Registry:   docker exec -it %s psql -U %s -d %s\n" \
    "$TR_DB_CONTAINER" "$TR_DB_USER" "$TR_DB_NAME"
  printf "    Tenant GL:  docker exec -it %s psql -U %s -d \"%s\"\n" \
    "$GL_DB_CONTAINER" "$GL_DB_USER" "$GL_TENANT_DB"
  printf "    Tenant AR:  docker exec -it 7d-ar-postgres psql -U ar_user -d \"tenant_%s_ar_db\"\n" \
    "$DEV_TENANT_UUID"
  echo ""
  echo "  DB access — host psql (port-forwarded)"
  printf "    Auth:       psql postgresql://%s:auth_pass@localhost:5433/%s\n" \
    "$AUTH_DB_USER" "$AUTH_DB_NAME"
  printf "    Registry:   psql postgresql://%s:tenant_registry_pass@localhost:5441/%s\n" \
    "$TR_DB_USER" "$TR_DB_NAME"
  echo "============================================================"
}

# ── Main ──────────────────────────────────────────────────

echo ""
echo "  7D Solutions Platform — Dev Seeder"
echo "  Tenant: $DEV_TENANT_SLUG  ($DEV_TENANT_UUID)"
echo ""

step1_preflight
step2_platform_admin
step3_provision_tenant
step4_gl_period
step5_ar_demo
print_summary
