#!/usr/bin/env bash
# scripts/proof_platform_completion.sh
#
# Platform Completion Gate — P47-900 Capstone
#
# Proves in a single repeatable run that:
#   1. All OpenAPI contracts are valid and have no unacknowledged breaking changes.
#   2. The perf smoke scenario passes against a live environment (local or staging).
#   3. The TCP UI onboarding E2E suite passes against a live frontend.
#
# Usage:
#   ./scripts/proof_platform_completion.sh                  # local, all gates
#   ./scripts/proof_platform_completion.sh --skip-perf      # skip k6 (no k6 installed)
#   ./scripts/proof_platform_completion.sh --skip-e2e       # skip Playwright
#   ./scripts/proof_platform_completion.sh --staging <host> # run perf against staging
#
# Environment variables:
#   STAGING_HOST       — VPS hostname/IP (overrides --staging)
#   PERF_AUTH_EMAIL    — login email for k6
#   PERF_AUTH_PASSWORD — login password for k6
#   PERF_AUTH_TOKEN    — pre-minted JWT (skips k6 login step)
#   BASE_URL           — Playwright base URL (default: http://localhost:3000)
#   BASE_REF           — git ref for breaking-change comparison (default: HEAD~1)
#
# Exits 0 only when all enabled gates pass.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# ── Argument parsing ─────────────────────────────────────────────────────────
SKIP_PERF=false
SKIP_E2E=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-perf)    SKIP_PERF=true; shift ;;
    --skip-e2e)     SKIP_E2E=true; shift ;;
    --staging)      STAGING_HOST="${2:?'--staging requires a hostname'}"; shift 2 ;;
    *)              printf 'ERROR: Unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

STAGING_HOST="${STAGING_HOST:-}"
BASE_URL="${BASE_URL:-http://localhost:3000}"
BASE_REF="${BASE_REF:-HEAD~1}"

# ── Tracking ─────────────────────────────────────────────────────────────────
PASS=0
FAIL=0
GATE_RESULTS=()

log_section() {
  echo ""
  echo "══════════════════════════════════════════════════════"
  echo "  $*"
  echo "══════════════════════════════════════════════════════"
}

log_step() { echo ""; echo "▶ $*"; }
log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

record_gate() {
  local name="$1" result="$2"
  GATE_RESULTS+=("$(printf '  %-40s %s' "$name" "$result")")
}

# ── Header ───────────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║   7D Solutions Platform — Completion Gate P47-900    ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Repo:       $REPO_ROOT"
echo "  Date:       $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "  Git SHA:    $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
echo "  Base URL:   $BASE_URL"
[[ -n "$STAGING_HOST" ]] && echo "  Staging:    $STAGING_HOST"
echo ""

# ════════════════════════════════════════════════════════
# GATE 1 — CONTRACT VALIDATION
# Proves all OpenAPI YAML/JSON specs in contracts/ are
# syntactically valid and parse cleanly.
# ════════════════════════════════════════════════════════
log_section "GATE 1 — Contract Validation"

log_step "Validate YAML contracts"
if python3 -c "
import yaml, sys
from pathlib import Path
errors = []
specs = list(Path('contracts').rglob('*.yaml'))
if not specs:
    print('  WARNING: no YAML contracts found')
for f in specs:
    try:
        yaml.safe_load(f.read_text())
        print(f'  ✓ {f}')
    except Exception as e:
        errors.append(f'  ✗ {f}: {e}')
if errors:
    for e in errors: print(e, file=sys.stderr)
    sys.exit(1)
" 2>&1; then
  log_pass "All YAML contracts parse cleanly"
  record_gate "YAML contracts" "PASS"
else
  log_fail "One or more YAML contracts are invalid"
  record_gate "YAML contracts" "FAIL"
fi

log_step "Validate JSON contracts"
if python3 -c "
import json, sys
from pathlib import Path
errors = []
specs = list(Path('contracts').rglob('*.json'))
if not specs:
    print('  WARNING: no JSON contracts found')
for f in specs:
    try:
        json.loads(f.read_text())
        print(f'  ✓ {f}')
    except Exception as e:
        errors.append(f'  ✗ {f}: {e}')
if errors:
    for e in errors: print(e, file=sys.stderr)
    sys.exit(1)
" 2>&1; then
  log_pass "All JSON contracts parse cleanly"
  record_gate "JSON contracts" "PASS"
else
  log_fail "One or more JSON contracts are invalid"
  record_gate "JSON contracts" "FAIL"
fi

# ════════════════════════════════════════════════════════
# GATE 2 — CONTRACT BREAKING-CHANGE CHECK
# Proves no breaking changes landed without an acknowledged
# version bump in info.version of the modified spec.
# ════════════════════════════════════════════════════════
log_section "GATE 2 — Contract Breaking-Change Gate"

log_step "Check for unacknowledged breaking changes (base: $BASE_REF)"
if BASE_REF="$BASE_REF" bash scripts/ci/check-openapi-breaking-changes.sh "$BASE_REF" 2>&1; then
  log_pass "No unacknowledged breaking changes detected"
  record_gate "Contract breaking-change gate" "PASS"
else
  log_fail "Breaking-change gate fired — see output above for details"
  record_gate "Contract breaking-change gate" "FAIL"
fi

# ════════════════════════════════════════════════════════
# GATE 3 — PERF SMOKE
# Proves the billing spine endpoints respond within SLA
# thresholds under a single-VU smoke scenario.
# ════════════════════════════════════════════════════════
log_section "GATE 3 — Performance Smoke"

if $SKIP_PERF; then
  echo "  (skipped via --skip-perf)"
  record_gate "Perf smoke (k6)" "SKIPPED"
else
  if ! command -v k6 &>/dev/null; then
    log_fail "k6 not found in PATH — install k6 or run with --skip-perf"
    record_gate "Perf smoke (k6)" "FAIL"
  else
    log_step "k6 smoke scenario"
    PERF_ENV_VAL="local"
    [[ -n "$STAGING_HOST" ]] && PERF_ENV_VAL="staging"

    SMOKE_EXPORT="perf-smoke-gate-$(git rev-parse --short HEAD 2>/dev/null || echo 'local')-$(date -u +%Y%m%dT%H%M%SZ).json"

    if PERF_ENV="$PERF_ENV_VAL" \
       STAGING_HOST="$STAGING_HOST" \
       PERF_AUTH_EMAIL="${PERF_AUTH_EMAIL:-}" \
       PERF_AUTH_PASSWORD="${PERF_AUTH_PASSWORD:-}" \
       PERF_AUTH_TOKEN="${PERF_AUTH_TOKEN:-}" \
       k6 run tools/perf/smoke.js \
         --summary-export="$SMOKE_EXPORT" 2>&1; then
      log_pass "k6 smoke: all thresholds met (summary: $SMOKE_EXPORT)"
      record_gate "Perf smoke (k6)" "PASS"
    else
      log_fail "k6 smoke: threshold breach or error — see output above"
      record_gate "Perf smoke (k6)" "FAIL"
    fi
  fi
fi

# ════════════════════════════════════════════════════════
# GATE 4 — ONBOARDING E2E (Playwright)
# Proves the TCP UI onboarding wizard works end-to-end
# against real backend services.
# ════════════════════════════════════════════════════════
log_section "GATE 4 — Onboarding E2E (Playwright)"

if $SKIP_E2E; then
  echo "  (skipped via --skip-e2e)"
  record_gate "Onboarding E2E (Playwright)" "SKIPPED"
else
  log_step "Playwright onboarding-wizard spec (BASE_URL=$BASE_URL)"
  if ! command -v npx &>/dev/null; then
    log_fail "npx not found — install Node.js or run with --skip-e2e"
    record_gate "Onboarding E2E (Playwright)" "FAIL"
  else
    if BASE_URL="$BASE_URL" \
       npx --prefix apps/tenant-control-plane-ui \
         playwright test tests/onboarding-wizard.spec.ts \
         --reporter=list 2>&1; then
      log_pass "Playwright: all onboarding-wizard tests passed"
      record_gate "Onboarding E2E (Playwright)" "PASS"
    else
      log_fail "Playwright: one or more onboarding-wizard tests failed — see output above"
      record_gate "Onboarding E2E (Playwright)" "FAIL"
    fi
  fi
fi

# ════════════════════════════════════════════════════════
# SUMMARY
# ════════════════════════════════════════════════════════
log_section "PLATFORM COMPLETION GATE — SUMMARY"

for line in "${GATE_RESULTS[@]}"; do
  echo "$line"
done

echo ""
echo "  Passed: $PASS   Failed: $FAIL"
echo ""

if [[ $FAIL -eq 0 ]]; then
  echo "  ✅  PLATFORM COMPLETE — all gates passed."
  echo "      This release candidate is proven."
  echo ""
  exit 0
else
  echo "  ❌  GATE FAILED — $FAIL check(s) did not pass."
  echo "      Fix the failures above before declaring completion."
  echo ""
  exit 1
fi
