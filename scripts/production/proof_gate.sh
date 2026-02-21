#!/usr/bin/env bash
# proof_gate.sh — Production Proof Gate (P45-220)
#
# Single authoritative green-light for production readiness.
# Runs all four proof suites in sequence; all must pass for the gate to succeed.
#
#   1. smoke             — /healthz + /api/ready + data endpoints (curl via SSH)
#   2. isolation_check   — tenant A/B cross-tenant denial assertions (curl via SSH)
#   3. payment_verify    — invoice → webhook → posting (Tilled test mode, curl via SSH)
#   4. rollback_rehearsal — deployment history read + rollback preflight (SSH)
#
# Usage (local):
#   PROD_HOST=<host> TILLED_WEBHOOK_SECRET=<secret> bash scripts/production/proof_gate.sh
#   bash scripts/production/proof_gate.sh [--host HOST] [--secret SECRET] [--jwt JWT]
#
# Usage (CI — set env vars before calling):
#   PROD_HOST=... TILLED_WEBHOOK_SECRET=... bash scripts/production/proof_gate.sh
#
# Environment variables:
#   PROD_HOST               — VPS hostname or IP (required)
#   PROD_USER               — SSH deploy user (default: deploy)
#   PROD_SSH_PORT           — SSH port (default: 22)
#   PROD_REPO_PATH          — Repo checkout path on VPS (default: /opt/7d-platform)
#   TILLED_WEBHOOK_SECRET   — Tilled HMAC-SHA256 signing secret (required for payment suite)
#   SMOKE_STAFF_JWT         — Staff JWT for data assertions in smoke suite (optional)
#   PROOF_GATE_LOG_DIR      — Directory for per-suite log files (default: /tmp/proof_gate_logs)
#
# Exit code: 0 = all proofs passed, 1 = one or more proofs failed.

# Note: -e is intentionally omitted so all suites run even when one fails.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Argument parsing ────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)   PROD_HOST="$2";              shift 2 ;;
        --secret) TILLED_WEBHOOK_SECRET="$2";  shift 2 ;;
        --jwt)    SMOKE_STAFF_JWT="$2";        shift 2 ;;
        *) printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done

# Export so sub-scripts inherit these values.
export PROD_HOST="${PROD_HOST:-}"
export PROD_USER="${PROD_USER:-deploy}"
export PROD_SSH_PORT="${PROD_SSH_PORT:-22}"
export PROD_REPO_PATH="${PROD_REPO_PATH:-/opt/7d-platform}"
export TILLED_WEBHOOK_SECRET="${TILLED_WEBHOOK_SECRET:-}"
export SMOKE_STAFF_JWT="${SMOKE_STAFF_JWT:-}"

# ── Required environment validation ─────────────────────────────────────────────
if [[ -z "$PROD_HOST" ]]; then
    printf 'ERROR: PROD_HOST must be set (via env var or --host).\n' >&2
    printf '       Copy scripts/production/env.example → scripts/production/.env.production\n' >&2
    exit 1
fi

if [[ -z "$TILLED_WEBHOOK_SECRET" ]]; then
    printf 'WARNING: TILLED_WEBHOOK_SECRET is not set — payment_verify suite will fail.\n' >&2
    printf '         Set TILLED_WEBHOOK_SECRET (env var or --secret) to run the full payment proof.\n' >&2
fi

# ── Log directory setup ──────────────────────────────────────────────────────────
LOG_DIR="${PROOF_GATE_LOG_DIR:-/tmp/proof_gate_logs}"
mkdir -p "$LOG_DIR"
REPORT_FILE="${LOG_DIR}/proof_gate_report.txt"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# ── Display helpers ──────────────────────────────────────────────────────────────
rule()   { printf '══════════════════════════════════════════════════════════\n'; }
banner() { printf '\n'; rule; printf '  %s\n' "$*"; rule; }

# ── Suite runner ─────────────────────────────────────────────────────────────────
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

# ── Rollback rehearsal ───────────────────────────────────────────────────────────
# Read-only: shows deployment history and validates SSH connectivity used for
# rollback.  No containers are changed.  Operators run rollback_stack.sh --previous
# to execute an actual rollback.
run_rollback_rehearsal() {
    local log_file="${LOG_DIR}/rollback_rehearsal.log"
    local start_ts end_ts status

    banner "Suite: rollback_rehearsal"
    start_ts=$(date +%s)

    {
        printf 'Validating rollback infrastructure (SSH + deployment log)...\n'
        printf 'PROD_HOST:       %s\n' "$PROD_HOST"
        printf 'PROD_USER:       %s\n' "$PROD_USER"
        printf 'PROD_REPO_PATH:  %s\n' "$PROD_REPO_PATH"
        printf '\n'

        printf '=== Deployment history (last 10 entries) ===\n'
        bash "${SCRIPT_DIR}/rollback_stack.sh" --history
        status=$?

        if [[ $status -ne 0 ]]; then
            printf 'ERROR: Could not read deployment history from %s:%s/%s\n' \
                "$PROD_HOST" "$PROD_REPO_PATH" ".production-deployments" >&2
            printf '       Check SSH connectivity and that at least one deploy has run.\n' >&2
            return $status
        fi

        printf '\n'
        printf 'Rollback preflight: PASSED\n'
        printf '\n'
        printf 'To roll back production, run one of:\n'
        printf '  bash scripts/production/rollback_stack.sh --previous\n'
        printf '  bash scripts/production/rollback_stack.sh --tag <prior-tag>\n'
        printf '\n'
        printf 'After rollback, update deploy/production/MODULE-MANIFEST.md to reflect\n'
        printf 'the rolled-back tags and commit the change.\n'
        printf '\n'
        printf 'Rollback rehearsal PROVEN: SSH connectivity and deployment log confirmed.\n'
    } 2>&1 | tee "$log_file"
    status=${PIPESTATUS[0]}

    end_ts=$(date +%s)
    printf '\n[rollback_rehearsal] completed in %ds — exit %d\n' \
        "$((end_ts - start_ts))" "$status"

    return "$status"
}

# ── Run all four suites ──────────────────────────────────────────────────────────
SMOKE_STATUS=0
ISOLATION_STATUS=0
PAYMENT_STATUS=0
ROLLBACK_STATUS=0

run_suite "smoke"           "${SCRIPT_DIR}/smoke.sh"           || SMOKE_STATUS=$?
run_suite "isolation_check" "${SCRIPT_DIR}/isolation_check.sh" || ISOLATION_STATUS=$?
run_suite "payment_verify"  "${SCRIPT_DIR}/payment_verify.sh"  || PAYMENT_STATUS=$?
run_rollback_rehearsal                                          || ROLLBACK_STATUS=$?

# ── Compute overall gate result ──────────────────────────────────────────────────
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
GATE_STATUS=0
[[ $SMOKE_STATUS -ne 0 || $ISOLATION_STATUS -ne 0 || $PAYMENT_STATUS -ne 0 || $ROLLBACK_STATUS -ne 0 ]] && GATE_STATUS=1

result_label() { [[ "$1" -eq 0 ]] && printf 'PASS' || printf 'FAIL'; }

# ── Write summary report (stdout + report file) ──────────────────────────────────
{
    printf '\n'
    rule
    printf '  Production Proof Gate — SUMMARY\n'
    rule
    printf '  Host:     %s\n' "$PROD_HOST"
    printf '  Started:  %s\n' "$STARTED_AT"
    printf '  Finished: %s\n' "$FINISHED_AT"
    printf '\n'
    printf '  %-25s  %-6s  %s\n' "Suite" "Result" "Log"
    printf '  %-25s  %-6s  %s\n' \
        "─────────────────────────" "──────" "──────────────────────────────────────────────────"
    printf '  %-25s  %-6s  %s\n' \
        "smoke"              "$(result_label $SMOKE_STATUS)"    "${LOG_DIR}/smoke.log"
    printf '  %-25s  %-6s  %s\n' \
        "isolation_check"    "$(result_label $ISOLATION_STATUS)" "${LOG_DIR}/isolation_check.log"
    printf '  %-25s  %-6s  %s\n' \
        "payment_verify"     "$(result_label $PAYMENT_STATUS)"  "${LOG_DIR}/payment_verify.log"
    printf '  %-25s  %-6s  %s\n' \
        "rollback_rehearsal" "$(result_label $ROLLBACK_STATUS)" "${LOG_DIR}/rollback_rehearsal.log"
    printf '\n'
    printf '  Gate result: %s\n' "$(result_label $GATE_STATUS)"
    rule
} | tee "$REPORT_FILE"

printf '\nProof gate report written to: %s\n' "$REPORT_FILE"

exit $GATE_STATUS
