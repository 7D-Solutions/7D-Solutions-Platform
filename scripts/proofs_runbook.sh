#!/usr/bin/env bash
# 7D Platform Phase Proofs Runbook
# - Discovers running 7d-* containers
# - Runs ./scripts/cargo-slot.sh test per crate discovered under modules/* and platform/*
# - Captures /healthz, /api/ready, /metrics per service (no auth)
# - Runs platform_contracts tests
# - Checks NATS JetStream health via docker exec 7d-nats nats
# - Captures outbox-related metrics lines per service
#
# Idempotent: safe to re-run; overwrites per-run artifacts under proofs/<timestamp>/
#
# Usage:
#   bash scripts/proofs_runbook.sh
#   cp scripts/.env.test.proofs.example .env.test.proofs
#   source .env.test.proofs
#
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SLOT_SCRIPT="./scripts/cargo-slot.sh"

if [[ ! -x "$SLOT_SCRIPT" ]]; then
  echo "ERROR: slot script not found or not executable: $SLOT_SCRIPT" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "ERROR: docker not found in PATH" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq not found in PATH (required for JSON outputs)" >&2
  exit 1
fi

RUN_TS="$(date -u +"%Y%m%dT%H%M%SZ")"
PROOFS_DIR="$ROOT_DIR/proofs/$RUN_TS"

mkdir -p "$PROOFS_DIR"/{cross-phase,tests,services,nats}
mkdir -p "$PROOFS_DIR/services"/{health,ready,metrics,outbox}
mkdir -p "$PROOFS_DIR/tests"/{modules,platform}

LOG="$PROOFS_DIR/runbook.log"
SUMMARY="$PROOFS_DIR/summary.txt"

touch "$LOG" "$SUMMARY"

log() {
  local msg="$1"
  echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] $msg" | tee -a "$LOG"
}

warn() {
  local msg="$1"
  echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] WARN: $msg" | tee -a "$LOG" >&2
}

err() {
  local msg="$1"
  echo "[$(date -u +"%Y-%m-%dT%H:%M:%SZ")] ERROR: $msg" | tee -a "$LOG" >&2
}

on_error() {
  local exit_code=$?
  local line_no=$1
  err "Runbook aborted (exit=$exit_code) at line $line_no. See $LOG"
  exit "$exit_code"
}
trap 'on_error $LINENO' ERR

# ----------------------------
# Helpers
# ----------------------------

safe_run() {
  # safe_run <label> <outfile> <command...>
  # Runs command, captures stdout+stderr to outfile with metadata header/footer.
  # Returns the command's exit code WITHOUT killing the script.
  local label="$1"; shift
  local outfile="$1"; shift
  local tmp="${outfile}.tmp"

  log "$label"

  local rc=0
  {
    echo "## $label"
    echo "## started_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "## cmd=$*"
    echo
  } >"$tmp" 2>&1

  "$@" >>"$tmp" 2>&1 || rc=$?

  {
    echo
    echo "## exit_code=$rc"
    echo "## finished_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  } >>"$tmp" 2>&1

  mv "$tmp" "$outfile"
  return "$rc"
}

curl_json() {
  # curl_json <url> <outfile_json>
  local url="$1"
  local out="$2"
  local tmp="${out}.tmp"

  local body
  body="$(curl -sS --max-time 5 "$url" 2>/dev/null || true)"
  if echo "$body" | jq -e . >/dev/null 2>&1; then
    echo "$body" | jq . >"$tmp"
  else
    jq -n --arg url "$url" --arg raw "$body" '{"url":$url,"raw":$raw}' >"$tmp"
  fi
  mv "$tmp" "$out"
}

curl_text() {
  # curl_text <url> <outfile_txt>
  local url="$1"
  local out="$2"
  local tmp="${out}.tmp"
  {
    echo "## url=$url"
    echo "## fetched_at_utc=$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo
    curl -sS --max-time 5 "$url" 2>/dev/null
  } >"$tmp" 2>&1 || true
  mv "$tmp" "$out"
}

container_host_port() {
  # container_host_port <container_name>
  # Returns first published TCP host port (lowest numeric).
  local c="$1"

  local ports
  ports="$(docker inspect "$c" --format '{{json .NetworkSettings.Ports}}' 2>/dev/null || true)"
  if [[ -z "$ports" || "$ports" == "null" ]]; then
    echo ""
    return 0
  fi

  echo "$ports" \
    | jq -r 'to_entries[]
        | .value
        | if . == null then empty else .[]?.HostPort end' 2>/dev/null \
    | awk 'NF' \
    | sort -n \
    | head -n 1
}

discover_services() {
  docker ps --format '{{.Names}}' | grep -E '^7d-' || true
}

discover_crates() {
  # Finds Cargo.toml under modules/* and platform/* and prints: <crate_name>\t<manifest_path>
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

crate_test_env_cmd() {
  # crate_test_env_cmd <crate_name>
  # Echoes an env-prefixed command fragment for crates that need explicit DB wiring.
  local crate="$1"
  case "$crate" in
    gl-rs)
      echo "DATABASE_URL=${DATABASE_URL_GL:-postgres://gl_user:gl_pass@localhost:5438/gl_db}"
      ;;
    inventory-rs)
      echo "DATABASE_URL=${DATABASE_URL_INVENTORY:-postgres://inventory_user:inventory_pass@localhost:5442/inventory_db}"
      ;;
    shipping-receiving-rs)
      echo "DATABASE_URL=${DATABASE_URL_SHIPPING_RECEIVING:-postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db}"
      ;;
    auth-rs)
      echo "DATABASE_URL=${DATABASE_URL_AUTH:-postgres://auth_user:auth_pass@localhost:5433/auth_db}"
      ;;
    treasury)
      echo "DATABASE_URL=${DATABASE_URL_TREASURY:-postgres://treasury_user:treasury_pass@localhost:5444/treasury_db} RUST_TEST_THREADS=${RUST_TEST_THREADS_TREASURY:-1}"
      ;;
    *)
      echo ""
      ;;
  esac
}

# ----------------------------
# Run: Readiness/Health/Metrics sweep
# ----------------------------
log "Runbook start. proofs_dir=$PROOFS_DIR"

services=()
while IFS= read -r s; do
  [[ -n "$s" ]] && services+=("$s")
done < <(discover_services)

log "Discovered ${#services[@]} running 7d-* containers"

service_status_json="$PROOFS_DIR/services/service_status.json"
echo '{"services":[]}' | jq --arg ts "$RUN_TS" '. + {run_ts:$ts}' >"$service_status_json"

for c in "${services[@]}"; do
  host_port="$(container_host_port "$c")"
  if [[ -z "${host_port:-}" ]]; then
    warn "No published TCP port found for $c (non-HTTP or not published). Recording as non_http."
    tmp="$(mktemp)"
    jq --arg c "$c" '.services += [{"container":$c,"host_port":null,"base_url":null,"http":false}]' \
      "$service_status_json" >"$tmp" && mv "$tmp" "$service_status_json"
    continue
  fi

  base_url="http://localhost:${host_port}"

  tmp="$(mktemp)"
  jq --arg c "$c" --arg hp "$host_port" --arg url "$base_url" \
    '.services += [{"container":$c,"host_port":($hp|tonumber),"base_url":$url,"http":true}]' \
    "$service_status_json" >"$tmp" && mv "$tmp" "$service_status_json"

  curl_json "${base_url}/healthz" "$PROOFS_DIR/services/health/${c}.json"
  curl_json "${base_url}/api/ready" "$PROOFS_DIR/services/ready/${c}.json"
  curl_text "${base_url}/metrics" "$PROOFS_DIR/services/metrics/${c}.txt"

  # Outbox slice (best-effort grep)
  grep -E -i 'outbox|publish|publisher|jetstream|nats|dlq' "$PROOFS_DIR/services/metrics/${c}.txt" \
    >"$PROOFS_DIR/services/outbox/${c}.txt" 2>/dev/null || true
done

# ----------------------------
# Run: Contract tests
# ----------------------------
contracts_out="$PROOFS_DIR/cross-phase/platform-contracts.txt"
contracts_rc=0
safe_run "platform_contracts: cargo-slot test" "$contracts_out" \
  "$SLOT_SCRIPT" test -p platform_contracts -- --nocapture || contracts_rc=$?

# ----------------------------
# Run: NATS JetStream health (via docker exec 7d-nats nats)
# ----------------------------
nats_check_out="$PROOFS_DIR/nats/nats_server_check.txt"
nats_streams_out="$PROOFS_DIR/nats/nats_stream_ls.txt"
nats_rc=0

if docker ps --format '{{.Names}}' | grep -qx '7d-nats'; then
  safe_run "nats: server check" "$nats_check_out" docker exec 7d-nats nats server check || nats_rc=$?
  safe_run "nats: stream ls" "$nats_streams_out" docker exec 7d-nats nats stream ls || nats_rc=$?
else
  warn "7d-nats container not found; skipping NATS checks"
  echo "SKIPPED: 7d-nats not found" >"$nats_check_out"
  echo "SKIPPED: 7d-nats not found" >"$nats_streams_out"
  nats_rc=2
fi

# ----------------------------
# Run: Cargo tests for all discovered crates
# ----------------------------
declare -A crate_manifest
declare -A crate_rc

while IFS=$'\t' read -r name toml; do
  crate_manifest["$name"]="$toml"
done < <(discover_crates)

log "Discovered ${#crate_manifest[@]} crates under modules/* and platform/*"

# Run all crate tests serially (slot system handles concurrency constraints)
for name in $(echo "${!crate_manifest[@]}" | tr ' ' '\n' | sort); do
  manifest="${crate_manifest[$name]}"
  # Store output by location
  if [[ "$manifest" == *"/modules/"* ]]; then
    out="$PROOFS_DIR/tests/modules/${name}.txt"
  else
    out="$PROOFS_DIR/tests/platform/${name}.txt"
  fi

  rc=0
  env_prefix="$(crate_test_env_cmd "$name")"
  if [[ -n "$env_prefix" ]]; then
    safe_run "crate test: $name" "$out" bash -lc "$env_prefix \"$SLOT_SCRIPT\" test -p \"$name\" -- --nocapture" || rc=$?
  else
    safe_run "crate test: $name" "$out" "$SLOT_SCRIPT" test -p "$name" -- --nocapture || rc=$?
  fi
  crate_rc["$name"]="$rc"
done

# ----------------------------
# Summary
# ----------------------------
log "Building summary"

pass_crates=0
fail_crates=0

{
  echo "7D Platform Proofs Runbook Summary"
  echo "=================================="
  echo "run_ts_utc=$RUN_TS"
  echo "proofs_dir=$PROOFS_DIR"
  echo
  echo "== Services discovered =="
  echo "count=${#services[@]}"
  echo "service_status_json=$service_status_json"
  echo
  echo "== Contract tests =="
  echo "platform_contracts_exit_code=$contracts_rc"
  echo "platform_contracts_output=$contracts_out"
  echo
  echo "== NATS checks =="
  echo "nats_exit_code=$nats_rc"
  echo "nats_server_check=$nats_check_out"
  echo "nats_stream_ls=$nats_streams_out"
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
  echo
  echo "NOTE: This runbook does NOT execute tenant meta-tests or performance benchmarks (handled separately)."
  echo "NOTE: Readiness checks are unauthenticated: /healthz, /api/ready, /metrics only."
} >>"$SUMMARY"

log "Runbook complete. Summary at $SUMMARY"
cat "$SUMMARY"

# Exit non-zero if critical checks failed
if [[ "$contracts_rc" -ne 0 || "$nats_rc" -ne 0 || "$fail_crates" -ne 0 ]]; then
  err "One or more checks failed: contracts_rc=$contracts_rc nats_rc=$nats_rc crates_fail=$fail_crates"
  exit 2
fi

exit 0
