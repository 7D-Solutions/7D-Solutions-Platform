#!/usr/bin/env bash
# Nightly financial invariant reconciliation runner
#
# Runs all configured module invariant checks and writes Prometheus metrics.
# Intended to be run as a nightly cron job (e.g., 02:00 UTC daily).
#
# Environment variables (all optional — modules without a URL are skipped):
#   AR_DATABASE_URL           PostgreSQL URL for the AR module
#   AP_DATABASE_URL           PostgreSQL URL for the AP module
#   GL_DATABASE_URL           PostgreSQL URL for the GL module
#   INVENTORY_DATABASE_URL    PostgreSQL URL for the Inventory module
#   BOM_DATABASE_URL          PostgreSQL URL for the BOM module
#   PRODUCTION_DATABASE_URL   PostgreSQL URL for the Production module
#   RECON_METRICS_OUTPUT      Where to write .prom file (default: stdout via "-")
#
# Exit codes:
#   0  All checks passed
#   1  One or more invariant violations found
#   2  Configuration error
#   3  Database connectivity error

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BINARY="${PROJECT_ROOT}/target/debug/reconciliation"

# Fall back to writing metrics to stdout if no output dir configured
export RECON_METRICS_OUTPUT="${RECON_METRICS_OUTPUT:--}"

# Try to find the built binary. If not found, build it first.
# NOTE: cargo-slot.sh builds into target-slot-N/ and nukes that dir on exit,
# so the binary never survives at target/debug/. We bypass the slot shim and
# build directly into target/ using the real cargo binary.
if [[ ! -f "${BINARY}" ]]; then
  echo "[recon] Binary not found at ${BINARY}. Building..." >&2
  # Find real cargo, bypassing the workspace bin/cargo shim
  _REAL_CARGO=""
  for _c in "$HOME/.cargo/bin/cargo" /usr/local/bin/cargo /opt/homebrew/bin/cargo /usr/bin/cargo; do
    if [[ -x "$_c" ]]; then _REAL_CARGO="$_c"; break; fi
  done
  if [[ -z "$_REAL_CARGO" ]]; then
    echo "[recon] ERROR: cargo binary not found" >&2
    exit 2
  fi
  CARGO_TARGET_DIR="${PROJECT_ROOT}/target" "$_REAL_CARGO" build -p reconciliation >&2
fi

echo "[recon] Starting reconciliation run at $(date -u +%Y-%m-%dT%H:%M:%SZ)" >&2

"${BINARY}" "$@"
EXIT_CODE=$?

if [[ ${EXIT_CODE} -eq 0 ]]; then
  echo "[recon] PASSED — all invariants satisfied" >&2
elif [[ ${EXIT_CODE} -eq 1 ]]; then
  echo "[recon] FAILED — invariant violations detected (see logs above)" >&2
elif [[ ${EXIT_CODE} -eq 2 ]]; then
  echo "[recon] ERROR — no database URLs configured" >&2
else
  echo "[recon] ERROR — database connectivity failure (exit ${EXIT_CODE})" >&2
fi

exit ${EXIT_CODE}
