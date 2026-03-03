#!/usr/bin/env bash
# proof-runbook-ci.sh — CI-adapted proof runbook
#
# Produces the same artifact structure as scripts/proofs_runbook.sh but runs
# in CI without requiring a full Docker-based environment.  Specifically:
#
#   proofs/<run_ts>/
#     summary.txt          – machine-readable summary (same format as local runbook)
#     runbook.log          – timestamped log
#     cross-phase/         – contract test output
#     tests/modules/*.txt  – per-crate test output (modules/)
#     tests/platform/*.txt – per-crate test output (platform/)
#     nats/                – NATS health check output (when NATS_URL is set)
#
# Gates (pipeline fails if any of these fail):
#   1. All workspace crate tests pass
#   2. Platform contract tests pass
#   3. NATS server health check passes (if NATS_URL is set)
#
# Usage:
#   bash scripts/ci/proof-runbook-ci.sh
#
# Environment:
#   PROOFS_OUTPUT_DIR  – override artifact root (default: $REPO_ROOT/proofs)
#   NATS_URL           – NATS connection URL for health check (optional)
#   DATABASE_URL       – default Postgres URL for crates that need one
#   *_DATABASE_URL     – per-crate overrides (see crate_test_env below)
#
set -Euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

RUN_TS="$(date -u +"%Y%m%dT%H%M%SZ")"
PROOFS_ROOT="${PROOFS_OUTPUT_DIR:-$ROOT_DIR/proofs}"
PROOFS_DIR="$PROOFS_ROOT/$RUN_TS"

mkdir -p "$PROOFS_DIR"/{cross-phase,nats}
mkdir -p "$PROOFS_DIR/tests"/{modules,platform}

LOG="$PROOFS_DIR/runbook.log"
SUMMARY="$PROOFS_DIR/summary.txt"
touch "$LOG" "$SUMMARY"

log() { echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] $1" | tee -a "$LOG"; }
warn() { echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] WARN: $1" | tee -a "$LOG" >&2; }
err()  { echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] ERROR: $1" | tee -a "$LOG" >&2; }

# ── Crate discovery ──────────────────────────────────────────────────────────
discover_crates() {
  local roots=("$ROOT_DIR/modules" "$ROOT_DIR/platform")
  for r in "${roots[@]}"; do
    [[ -d "$r" ]] || continue
    while IFS= read -r -d '' toml; do
      local name
      name="$(awk '
        BEGIN{inpkg=0}
        /^\[package\]/{inpkg=1; next}
        /^\[/{if($0!="[package]") inpkg=0}
        inpkg && $1=="name" && $2=="="{
          gsub(/"/,"",$3); print $3; exit
        }' "$toml" | tr -d '\r')"
      if [[ -n "$name" ]]; then
        printf "%s\t%s\n" "$name" "$toml"
      else
        warn "Could not parse crate name from $toml"
      fi
    done < <(find "$r" -maxdepth 3 -name Cargo.toml -print0 2>/dev/null)
  done | sort -u
}

# ── Per-crate env overrides ──────────────────────────────────────────────────
crate_test_env() {
  local crate="$1"
  local default_db="${DATABASE_URL:-}"
  case "$crate" in
    gl-rs)              echo "DATABASE_URL=${GL_DATABASE_URL:-$default_db}" ;;
    inventory-rs)       echo "DATABASE_URL=${INVENTORY_DATABASE_URL:-$default_db}" ;;
    shipping-receiving-rs) echo "DATABASE_URL=${SHIPPING_RECEIVING_DATABASE_URL:-$default_db}" ;;
    fixed-assets)       echo "DATABASE_URL=${FIXED_ASSETS_DATABASE_URL:-$default_db}" ;;
    subscriptions-rs)   echo "DATABASE_URL=${SUBSCRIPTIONS_DATABASE_URL:-$default_db}" ;;
    auth-rs)            echo "DATABASE_URL=${AUTH_DATABASE_URL:-$default_db} RUST_TEST_THREADS=1" ;;
    treasury)           echo "DATABASE_URL=${TREASURY_DATABASE_URL:-$default_db} RUST_TEST_THREADS=1" ;;
    audit)              echo "AUDIT_DATABASE_URL=${AUDIT_DATABASE_URL:-$default_db}" ;;
    projections)        echo "PROJECTIONS_DATABASE_URL=${PROJECTIONS_DATABASE_URL:-$default_db}" ;;
    tenant-registry)    echo "TENANT_REGISTRY_DATABASE_URL=${TENANT_REGISTRY_DATABASE_URL:-$default_db}" ;;
    *)                  echo "" ;;
  esac
}

# ── Safe runner (captures output + exit code) ────────────────────────────────
safe_run() {
  local label="$1"; shift
  local outfile="$1"; shift
  local rc=0
  {
    echo "## $label"
    echo "## started_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "## cmd=$*"
    echo
  } >"$outfile"
  "$@" >>"$outfile" 2>&1 || rc=$?
  {
    echo
    echo "## exit_code=$rc"
    echo "## finished_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  } >>"$outfile"
  return "$rc"
}

# ── Phase 1: Contract tests ─────────────────────────────────────────────────
log "Runbook start (CI mode). proofs_dir=$PROOFS_DIR"

contracts_out="$PROOFS_DIR/cross-phase/platform-contracts.txt"
contracts_rc=0
log "Running platform contract tests"
safe_run "platform_contracts: cargo test" "$contracts_out" \
  cargo test -p platform_contracts -- --nocapture || contracts_rc=$?
log "Contract tests: exit_code=$contracts_rc"

# ── Phase 2: NATS health check ──────────────────────────────────────────────
nats_check_out="$PROOFS_DIR/nats/nats_server_check.txt"
nats_streams_out="$PROOFS_DIR/nats/nats_stream_ls.txt"
nats_consumers_out="$PROOFS_DIR/nats/nats_consumers.txt"
nats_rc=0

NATS_MONITOR_URL="${NATS_MONITOR_URL:-}"
if [[ -z "$NATS_MONITOR_URL" && -n "${NATS_URL:-}" ]]; then
  nats_host="${NATS_URL#nats://}"
  nats_host="${nats_host%%:*}"
  NATS_MONITOR_URL="http://${nats_host}:8222"
fi

if [[ -n "$NATS_MONITOR_URL" ]]; then
  log "Checking NATS health at $NATS_MONITOR_URL"
  safe_run "nats: server health check ($NATS_MONITOR_URL)" "$nats_check_out" \
    bash -c "
      echo '=== /healthz ==='
      curl -sS --max-time 5 '${NATS_MONITOR_URL}/healthz'
      echo
      echo '=== /varz ==='
      curl -sS --max-time 5 '${NATS_MONITOR_URL}/varz'
    " || nats_rc=$?

  safe_run "nats: stream listing ($NATS_MONITOR_URL)" "$nats_streams_out" \
    bash -c "
      curl -sS --max-time 5 '${NATS_MONITOR_URL}/jsz?streams=true' 2>/dev/null || echo 'no streams'
    " || nats_rc=$?

  safe_run "nats: consumer listing ($NATS_MONITOR_URL)" "$nats_consumers_out" \
    bash -c "
      curl -sS --max-time 5 '${NATS_MONITOR_URL}/jsz?consumers=true' 2>/dev/null || echo 'no consumers'
    " || nats_rc=$?

  log "NATS checks: exit_code=$nats_rc"
else
  log "NATS_URL not set — skipping NATS checks"
  echo "SKIPPED: NATS_URL not set" >"$nats_check_out"
  echo "SKIPPED: NATS_URL not set" >"$nats_streams_out"
  echo "SKIPPED: NATS_URL not set" >"$nats_consumers_out"
fi

# ── Phase 3: Per-crate tests ────────────────────────────────────────────────
declare -A crate_manifest
declare -A crate_rc

while IFS=$'\t' read -r name toml; do
  crate_manifest["$name"]="$toml"
done < <(discover_crates)

log "Discovered ${#crate_manifest[@]} crates under modules/* and platform/*"

for name in $(echo "${!crate_manifest[@]}" | tr ' ' '\n' | sort); do
  manifest="${crate_manifest[$name]}"
  if [[ "$manifest" == *"/modules/"* ]]; then
    out="$PROOFS_DIR/tests/modules/${name}.txt"
  else
    out="$PROOFS_DIR/tests/platform/${name}.txt"
  fi

  rc=0
  env_prefix="$(crate_test_env "$name")"
  if [[ -n "$env_prefix" ]]; then
    safe_run "crate test: $name" "$out" \
      bash -c "env $env_prefix cargo test -p \"$name\" -- --nocapture" || rc=$?
  else
    safe_run "crate test: $name" "$out" \
      cargo test -p "$name" -- --nocapture || rc=$?
  fi
  crate_rc["$name"]="$rc"
  log "crate test: $name → exit_code=$rc"
done

# ── Summary ──────────────────────────────────────────────────────────────────
log "Building summary"

pass_crates=0
fail_crates=0

{
  echo "7D Platform Proofs Runbook Summary (CI)"
  echo "======================================="
  echo "run_ts_utc=$RUN_TS"
  echo "proofs_dir=$PROOFS_DIR"
  echo "mode=ci"
  echo
  echo "== Contract tests =="
  echo "platform_contracts_exit_code=$contracts_rc"
  echo "platform_contracts_output=$contracts_out"
  echo
  echo "== NATS checks =="
  echo "nats_exit_code=$nats_rc"
  echo "nats_server_check=$nats_check_out"
  echo "nats_stream_ls=$nats_streams_out"
  echo "nats_consumers=$nats_consumers_out"
  echo
  echo "== Crate tests =="
} >"$SUMMARY"

{
  printf "%-40s %s\n" "crate" "exit_code"
  printf "%-40s %s\n" "-----" "---------"
  for name in $(echo "${!crate_rc[@]}" | tr ' ' '\n' | sort); do
    printf "%-40s %s\n" "$name" "${crate_rc[$name]}"
  done
} >>"$SUMMARY"

for name in "${!crate_rc[@]}"; do
  if [[ "${crate_rc[$name]}" -eq 0 ]]; then
    pass_crates=$((pass_crates+1))
  else
    fail_crates=$((fail_crates+1))
  fi
done

{
  echo
  echo "== Totals =="
  echo "crates_pass=$pass_crates"
  echo "crates_fail=$fail_crates"
  echo "crates_total=$((pass_crates+fail_crates))"
  echo "contracts_pass=$( [[ $contracts_rc -eq 0 ]] && echo true || echo false )"
  echo "nats_pass=$( [[ $nats_rc -eq 0 ]] && echo true || echo false )"
} >>"$SUMMARY"

log "Runbook complete. Summary at $SUMMARY"
cat "$SUMMARY"

# ── Write to GITHUB_OUTPUT for artifact upload step ──────────────────────────
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "proofs_dir=$PROOFS_DIR" >>"$GITHUB_OUTPUT"
  echo "run_ts=$RUN_TS" >>"$GITHUB_OUTPUT"
fi

# ── Exit non-zero if any gate failed ─────────────────────────────────────────
if [[ "$contracts_rc" -ne 0 || "$nats_rc" -ne 0 || "$fail_crates" -ne 0 ]]; then
  err "PROOF GATE FAILED: contracts_rc=$contracts_rc nats_rc=$nats_rc crates_fail=$fail_crates"
  exit 1
fi

log "All proof gates passed"
exit 0
