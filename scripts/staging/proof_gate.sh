#!/usr/bin/env bash
# proof_gate.sh — Phase 43 Staging Proof Gate
#
# Runs all three staging proof suites in sequence and produces an authoritative
# pass/fail summary. All suites must pass for the gate to succeed.
#
#   1. smoke.sh           — /healthz + /api/ready + TCP UI + data endpoints
#   2. isolation_check.sh — tenant A/B cross-tenant denial assertions
#   3. payment_loop.sh    — invoice → webhook → posting + idempotency proof
#
# Usage (local):
#   bash scripts/staging/proof_gate.sh [--host HOST] [--secret SECRET] [--jwt JWT]
#
# Usage (CI — set env vars before calling):
#   STAGING_HOST=... TILLED_WEBHOOK_SECRET=... bash scripts/staging/proof_gate.sh
#
# Environment variables:
#   STAGING_HOST            — VPS hostname or IP (required)
#   TILLED_WEBHOOK_SECRET   — Tilled HMAC-SHA256 signing secret (required for payment loop)
#   SMOKE_STAFF_JWT         — Staff JWT for data assertions in smoke suite (optional)
#   PROOF_GATE_LOG_DIR      — Directory for per-suite log files (default: /tmp/proof_gate_logs)
#
# Exit code: 0 = all proofs passed, 1 = one or more proofs failed.

# Note: -e is intentionally omitted so all three suites run even when one fails.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Argument parsing ───────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)   STAGING_HOST="$2";          shift 2 ;;
        --secret) TILLED_WEBHOOK_SECRET="$2"; shift 2 ;;
        --jwt)    SMOKE_STAFF_JWT="$2";       shift 2 ;;
        *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done

# Export so sub-scripts inherit these values.
export STAGING_HOST="${STAGING_HOST:-}"
export TILLED_WEBHOOK_SECRET="${TILLED_WEBHOOK_SECRET:-}"
export SMOKE_STAFF_JWT="${SMOKE_STAFF_JWT:-}"

# ── Required environment validation ───────────────────────────────────────────
if [[ -z "$STAGING_HOST" ]]; then
    printf 'ERROR: STAGING_HOST must be set (via env var or --host).\n' >&2
    printf '       Copy scripts/staging/env.example → scripts/staging/.env.staging\n' >&2
    exit 1
fi

if [[ -z "$TILLED_WEBHOOK_SECRET" ]]; then
    printf 'WARNING: TILLED_WEBHOOK_SECRET is not set — payment_loop suite will fail.\n' >&2
    printf '         Set TILLED_WEBHOOK_SECRET (env var or --secret) to run the full payment proof.\n' >&2
fi

# ── Log directory setup ────────────────────────────────────────────────────────
LOG_DIR="${PROOF_GATE_LOG_DIR:-/tmp/proof_gate_logs}"
mkdir -p "$LOG_DIR"
REPORT_FILE="${LOG_DIR}/proof_gate_report.txt"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# ── Display helpers ────────────────────────────────────────────────────────────
rule()   { printf '══════════════════════════════════════════════════════════\n'; }
banner() { printf '\n'; rule; printf '  %s\n' "$*"; rule; }

# ── Suite runner ───────────────────────────────────────────────────────────────
# Runs a suite script and tees output to a per-suite log file.
# Returns the suite's exit code.
run_suite() {
    local name="$1" script="$2"
    local log_file="${LOG_DIR}/${name}.log"
    local start_ts end_ts local_status

    banner "Suite: ${name}"
    start_ts=$(date +%s)

    bash "$script" 2>&1 | tee "$log_file"
    local_status=${PIPESTATUS[0]}

    end_ts=$(date +%s)
    printf '\n[%s] completed in %ds — exit %d\n' \
        "$name" "$((end_ts - start_ts))" "$local_status"

    return "$local_status"
}

# ── Run all three suites ───────────────────────────────────────────────────────
SMOKE_STATUS=0
ISOLATION_STATUS=0
PAYMENT_STATUS=0

run_suite "smoke"           "${SCRIPT_DIR}/smoke.sh"           || SMOKE_STATUS=$?
run_suite "isolation_check" "${SCRIPT_DIR}/isolation_check.sh" || ISOLATION_STATUS=$?
run_suite "payment_loop"    "${SCRIPT_DIR}/payment_loop.sh"    || PAYMENT_STATUS=$?

# ── Compute overall gate result ────────────────────────────────────────────────
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
GATE_STATUS=0
[[ $SMOKE_STATUS -ne 0 || $ISOLATION_STATUS -ne 0 || $PAYMENT_STATUS -ne 0 ]] && GATE_STATUS=1

result_label() { [[ "$1" -eq 0 ]] && printf 'PASS' || printf 'FAIL'; }

# ── Write summary report (stdout + report file) ────────────────────────────────
{
    printf '\n'
    rule
    printf '  Phase 43 Staging Proof Gate — SUMMARY\n'
    rule
    printf '  Host:     %s\n' "$STAGING_HOST"
    printf '  Started:  %s\n' "$STARTED_AT"
    printf '  Finished: %s\n' "$FINISHED_AT"
    printf '\n'
    printf '  %-25s  %-6s  %s\n' "Suite" "Result" "Log"
    printf '  %-25s  %-6s  %s\n' \
        "─────────────────────────" "──────" "──────────────────────────────────────────────────"
    printf '  %-25s  %-6s  %s\n' \
        "smoke"           "$(result_label $SMOKE_STATUS)"     "${LOG_DIR}/smoke.log"
    printf '  %-25s  %-6s  %s\n' \
        "isolation_check" "$(result_label $ISOLATION_STATUS)" "${LOG_DIR}/isolation_check.log"
    printf '  %-25s  %-6s  %s\n' \
        "payment_loop"    "$(result_label $PAYMENT_STATUS)"   "${LOG_DIR}/payment_loop.log"
    printf '\n'
    printf '  Gate result: %s\n' "$(result_label $GATE_STATUS)"
    rule
} | tee "$REPORT_FILE"

printf '\nProof gate report written to: %s\n' "$REPORT_FILE"

exit $GATE_STATUS
