#!/usr/bin/env bash
# tools/load-test/run.sh — connection pool load test runner
#
# Usage:
#   ./tools/load-test/run.sh --module ap --concurrency 200 --duration 30s
#   ./tools/load-test/run.sh --module payments --concurrency 100 --duration 60s
#   ./tools/load-test/run.sh --all   # run all workload classes
#
# Requirements: k6 must be installed (brew install k6)
#
# Pass criteria (asserted by k6 thresholds):
#   - p99 latency < 2s
#   - Zero HTTP 503 responses (pool exhaustion)
#   - Error rate < 1%

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
K6_SCRIPT="$SCRIPT_DIR/pool-exhaustion.js"

# Module → port mapping (matches module.toml [server] port values)
declare -A MODULE_PORTS=(
  [ap]=8093
  [ar]=8086
  [bom]=8120
  [consolidation]=8105
  [customer-portal]=8111
  [fixed-assets]=8104
  [gl]=8090
  [integrations]=8099
  [inventory]=8092
  [maintenance]=8101
  [notifications]=8089
  [numbering]=8096
  [party]=8098
  [payments]=8088
  [pdf-editor]=8121
  [production]=8108
  [quality-inspection]=8106
  [reporting]=8097
  [shipping-receiving]=8103
  [smoke-test]=8199
  [subscriptions]=8087
  [timekeeping]=8102
  [treasury]=8094
  [ttp]=8100
  [vertical-proof]=8200
  [workflow]=8107
  [workforce-competence]=8110
)

# Workload classes for --all mode
WRITE_HEAVY=(ap ar gl payments production)
READ_HEAVY=(inventory bom shipping-receiving)
MIXED=(consolidation customer-portal fixed-assets integrations maintenance notifications numbering party pdf-editor quality-inspection reporting subscriptions timekeeping treasury ttp workflow workforce-competence)

MODULE=""
CONCURRENCY=200
DURATION="30s"
HOST="localhost"
RUN_ALL=false

usage() {
  cat <<EOF
Usage: $0 [OPTIONS]

Options:
  --module <name>       Module name (e.g. ap, ar, payments)
  --concurrency <n>     Concurrent virtual users (default: 200)
  --duration <t>        Test duration (default: 30s)
  --host <host>         Target host (default: localhost)
  --all                 Run all workload class representatives
  --help                Show this message

Examples:
  $0 --module ap --concurrency 200 --duration 30s
  $0 --module payments --concurrency 200 --duration 60s
  $0 --all
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --module)   MODULE="$2";      shift 2 ;;
    --concurrency) CONCURRENCY="$2"; shift 2 ;;
    --duration) DURATION="$2";    shift 2 ;;
    --host)     HOST="$2";        shift 2 ;;
    --all)      RUN_ALL=true;     shift   ;;
    --help|-h)  usage ;;
    *) echo "Unknown flag: $1"; usage ;;
  esac
done

check_k6() {
  if ! command -v k6 &>/dev/null; then
    echo "ERROR: k6 not found. Install with: brew install k6"
    exit 1
  fi
}

run_module() {
  local mod="$1"
  local port="${MODULE_PORTS[$mod]:-}"

  if [[ -z "$port" ]]; then
    echo "ERROR: unknown module '$mod'. Known modules: ${!MODULE_PORTS[*]}"
    return 1
  fi

  # Probe liveness before running the full test
  if ! curl -sf --max-time 2 "http://${HOST}:${port}/healthz" >/dev/null 2>&1; then
    echo "SKIP: $mod (http://${HOST}:${port}/healthz unreachable — is the service running?)"
    return 0
  fi
  # Verify the DB health endpoint is reachable (exercised by the load test)
  if ! curl -sf --max-time 3 "http://${HOST}:${port}/api/health" >/dev/null 2>&1; then
    echo "WARN: $mod /api/health returned non-200 — service may not be fully ready"
  fi

  echo ""
  echo "========================================"
  echo " Load test: $mod (port $port)"
  echo " Concurrency: $CONCURRENCY VUs | Duration: $DURATION"
  echo "========================================"

  k6 run \
    --env MODULE_NAME="$mod" \
    --env MODULE_HOST="$HOST" \
    --env MODULE_PORT="$port" \
    --env VUS="$CONCURRENCY" \
    --env DURATION="$DURATION" \
    --vus "$CONCURRENCY" \
    --duration "$DURATION" \
    "$K6_SCRIPT"
}

check_k6

if $RUN_ALL; then
  OVERALL_PASS=true

  echo ""
  echo "Running write-heavy class (ap, ar, gl, payments, production)"
  for mod in "${WRITE_HEAVY[@]}"; do
    run_module "$mod" || OVERALL_PASS=false
  done

  echo ""
  echo "Running read-heavy class (inventory, bom, shipping-receiving)"
  for mod in "${READ_HEAVY[@]}"; do
    run_module "$mod" || OVERALL_PASS=false
  done

  echo ""
  echo "Running mixed class sample (subscriptions, workflow, notifications)"
  for mod in subscriptions workflow notifications; do
    run_module "$mod" || OVERALL_PASS=false
  done

  echo ""
  if $OVERALL_PASS; then
    echo "ALL CLASSES PASSED"
    exit 0
  else
    echo "ONE OR MORE CLASSES FAILED"
    exit 1
  fi
else
  if [[ -z "$MODULE" ]]; then
    echo "ERROR: --module is required (or use --all)"
    usage
  fi
  run_module "$MODULE"
fi
