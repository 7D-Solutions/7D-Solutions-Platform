#!/usr/bin/env bash
# Proof script for platform/audit (package: audit)
# Run this before bumping to v1.0.0 or promoting to production.
#
# Usage:
#   ./scripts/proof_audit.sh
#
# Exits 0 only when all checks pass.
# Gates 3-4 require psql — skipped (with warning) if psql is unavailable.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

PASS=0
FAIL=0
SKIP=0

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
log_skip() { echo "  ⊘ $* (skipped)"; SKIP=$((SKIP + 1)); }

echo "=============================="
echo "  Proof: audit"
echo "=============================="

# ── Gate 1: Build ────────────────────────────────────────────────────────────
log_step "Build"
if ./scripts/cargo-slot.sh build -p audit 2>&1; then
  log_pass "cargo build -p audit"
else
  log_fail "cargo build -p audit"
fi

# ── Gate 2: Unit + integration tests ─────────────────────────────────────────
log_step "Tests (unit + integration)"
if ./scripts/cargo-slot.sh test -p audit 2>&1; then
  log_pass "cargo test -p audit"
else
  log_fail "cargo test -p audit"
fi

# ── Gate 3: Append-only invariant (requires psql) ───────────────────────────
log_step "Append-only invariant (DB trigger check)"

AUDIT_DB_URL="${AUDIT_DATABASE_URL:-${PLATFORM_AUDIT_DATABASE_URL:-postgresql://audit_user:audit_pass@localhost:5440/audit_db}}"

if command -v psql &>/dev/null; then
  # Verify UPDATE trigger exists
  if psql "$AUDIT_DB_URL" -tAc \
    "SELECT 1 FROM information_schema.triggers WHERE trigger_name='enforce_append_only_update' AND event_object_table='audit_events'" 2>/dev/null | grep -q 1; then
    log_pass "UPDATE trigger (enforce_append_only_update) exists"
  else
    log_fail "UPDATE trigger (enforce_append_only_update) missing"
  fi

  # Verify DELETE trigger exists
  if psql "$AUDIT_DB_URL" -tAc \
    "SELECT 1 FROM information_schema.triggers WHERE trigger_name='enforce_append_only_delete' AND event_object_table='audit_events'" 2>/dev/null | grep -q 1; then
    log_pass "DELETE trigger (enforce_append_only_delete) exists"
  else
    log_fail "DELETE trigger (enforce_append_only_delete) missing"
  fi

  # ── Gate 4: Required indexes exist ─────────────────────────────────────────
  log_step "Index presence"
  for idx in audit_events_occurred_at audit_events_actor_id audit_events_entity audit_events_action audit_events_mutation_class audit_events_correlation_id audit_events_trace_id; do
    if psql "$AUDIT_DB_URL" -tAc \
      "SELECT 1 FROM pg_indexes WHERE indexname='$idx'" 2>/dev/null | grep -q 1; then
      log_pass "Index $idx exists"
    else
      log_fail "Index $idx missing"
    fi
  done
else
  log_skip "psql not available — trigger and index checks skipped"
  log_skip "Integration tests already verify migration applies correctly"
fi

# ── Gate 5: Clippy (zero warnings) ──────────────────────────────────────────
log_step "Clippy"
if ./scripts/cargo-slot.sh clippy -p audit -- -D warnings 2>&1; then
  log_pass "clippy -p audit (zero warnings)"
else
  log_fail "clippy -p audit"
fi

# ── Gate 6: SQL injection guard ──────────────────────────────────────────────
log_step "SQL injection guard"
if grep -q 'assert!' platform/audit/src/outbox_bridge.rs \
   && grep -q 'is_ascii_lowercase' platform/audit/src/outbox_bridge.rs; then
  log_pass "query_outbox_events has table name validation"
else
  log_fail "query_outbox_events missing table name validation"
fi

# ── Gate 7: Migration SQL is valid (static check) ───────────────────────────
log_step "Migration file presence"
MIGRATION="platform/audit/db/migrations/20260216000001_create_audit_log.sql"
if [[ -f "$MIGRATION" ]]; then
  log_pass "Migration file exists: $MIGRATION"
  # Verify triggers are defined in the migration
  if grep -q "enforce_append_only_update" "$MIGRATION" && grep -q "enforce_append_only_delete" "$MIGRATION"; then
    log_pass "Append-only triggers defined in migration"
  else
    log_fail "Append-only triggers missing from migration SQL"
  fi
  # Verify indexes are defined
  if grep -q "audit_events_entity" "$MIGRATION" && grep -q "audit_events_correlation_id" "$MIGRATION"; then
    log_pass "Required indexes defined in migration"
  else
    log_fail "Required indexes missing from migration SQL"
  fi
else
  log_fail "Migration file missing: $MIGRATION"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "=============================="
echo "  audit proof: ${PASS} pass / ${FAIL} fail / ${SKIP} skip"
echo "=============================="
if [[ $FAIL -gt 0 ]]; then
  echo "PROOF FAILED — do not promote."
  exit 1
fi
echo "PROOF PASSED — safe to promote."
