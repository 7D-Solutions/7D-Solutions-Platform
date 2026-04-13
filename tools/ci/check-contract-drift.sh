#!/usr/bin/env bash
# tools/ci/check-contract-drift.sh
#
# CI gate: verify committed contracts/*/openapi.json matches what the live
# module code produces via the openapi_dump binary.
#
# Bead: bd-ke2f3 — GAP-13
#
# Usage:
#   ./tools/ci/check-contract-drift.sh              # check all modules
#   ./tools/ci/check-contract-drift.sh --ts-clients # also check TS client drift
#   ./tools/ci/check-contract-drift.sh --module ap  # single module
#
# Exit codes:
#   0 — no drift detected (warnings OK)
#   1 — drift detected in one or more modules
#
# How drift is detected:
#   1. Build the module's openapi_dump binary (via slot system or plain cargo)
#   2. Run it and capture JSON stdout
#   3. Normalise both outputs with `jq --sort-keys` to strip whitespace/order diffs
#   4. Diff the normalised forms — any diff is a FAIL
#
# For TS client drift (--ts-clients):
#   5. Run `openapi-typescript` against the fresh openapi.json
#   6. Diff output against clients/{module}/src/*.d.ts
#
# Modules without a committed contracts/{module}/openapi.json are skipped with
# a WARNING (not an error) so the gate grows coverage over time.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
CHECK_TS_CLIENTS=false
SINGLE_MODULE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ts-clients) CHECK_TS_CLIENTS=true; shift ;;
    --module)     SINGLE_MODULE="${2:?'--module requires a name'}"; shift 2 ;;
    *)            echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Module registry: "dir|cargo_package|contract_path"
# Only entries with an existing contracts/{module}/openapi.json are checked.
# ---------------------------------------------------------------------------
ALL_MODULES=(
  "ap|ap|contracts/ap/openapi.json"
  "bom|bom-rs|contracts/bom/openapi.json"
  "consolidation|consolidation|contracts/consolidation/openapi.json"
  "customer-portal|customer-portal|contracts/customer-portal/openapi.json"
  "fixed-assets|fixed-assets|contracts/fixed-assets/openapi.json"
  "integrations|integrations-rs|contracts/integrations/openapi.json"
  "numbering|numbering|contracts/numbering/openapi.json"
  "production|production-rs|contracts/production/openapi.json"
  "quality-inspection|quality-inspection-rs|contracts/quality-inspection/openapi.json"
  "reporting|reporting|contracts/reporting/openapi.json"
  "shipping-receiving|shipping-receiving-rs|contracts/shipping-receiving/openapi.json"
  "timekeeping|timekeeping|contracts/timekeeping/openapi.json"
  "treasury|treasury|contracts/treasury/openapi.json"
  "workflow|workflow|contracts/workflow/openapi.json"
  "workforce-competence|workforce-competence-rs|contracts/workforce-competence/openapi.json"
)

# Modules with openapi_dump but no JSON contract yet — warn only
WARN_MODULES=(ar gl inventory maintenance notifications party payments pdf-editor subscriptions ttp)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
PASS=0
FAIL=0
WARN=0

log_pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
log_fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }
log_warn() { echo "  ⚠ $*"; WARN=$((WARN + 1)); }
log_info() { echo "    $*"; }

# Resolve cargo invocation: use slot system if available, else plain cargo
if [[ -x "${REPO_ROOT}/scripts/cargo-slot.sh" ]]; then
  CARGO_CMD="${REPO_ROOT}/scripts/cargo-slot.sh"
else
  CARGO_CMD="cargo"
fi

# Normalise JSON: parse and re-emit with sorted keys for stable comparison
normalize_json() {
  jq --sort-keys '.' "$1"
}

# ---------------------------------------------------------------------------
# Filter to single module if requested
# ---------------------------------------------------------------------------
if [[ -n "$SINGLE_MODULE" ]]; then
  FILTERED=()
  for entry in "${ALL_MODULES[@]}"; do
    IFS='|' read -r dir _pkg _contract <<< "$entry"
    [[ "$dir" == "$SINGLE_MODULE" ]] && FILTERED+=("$entry")
  done
  if [[ ${#FILTERED[@]} -eq 0 ]]; then
    echo "ERROR: module '$SINGLE_MODULE' not in registry (or has no JSON contract)" >&2
    exit 1
  fi
  ALL_MODULES=("${FILTERED[@]}")
fi

# ---------------------------------------------------------------------------
# Header
# ---------------------------------------------------------------------------
echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║   7D Platform — OpenAPI Contract Drift Gate          ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Repo: ${REPO_ROOT}"
echo "  Date: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "  Cargo: ${CARGO_CMD}"
[[ -n "$SINGLE_MODULE" ]] && echo "  Module: ${SINGLE_MODULE}"
echo ""

# ---------------------------------------------------------------------------
# Gate 1 — OpenAPI JSON drift
# ---------------------------------------------------------------------------
echo "── Gate 1: OpenAPI JSON Contract Drift ──────────────────"
echo ""

DRIFT_MODULES=()

for entry in "${ALL_MODULES[@]}"; do
  IFS='|' read -r dir pkg contract <<< "$entry"
  echo "▶ ${dir} (${pkg})"

  # Verify contract file exists
  if [[ ! -f "${contract}" ]]; then
    log_warn "${dir}: contracts file missing at ${contract} — skipping"
    continue
  fi

  # Build openapi_dump binary
  BUILD_DIR="$(mktemp -d /tmp/openapi-drift-build.XXXXXX)"
  trap 'rm -rf "${BUILD_DIR}"' EXIT

  log_info "building openapi_dump -p ${pkg} …"
  if ! CARGO_TARGET_DIR="${BUILD_DIR}" \
       "${CARGO_CMD}" build --bin openapi_dump -p "${pkg}" \
       --quiet 2>/tmp/openapi-drift-build-stderr-$$.txt; then
    log_fail "${dir}: build failed"
    cat /tmp/openapi-drift-build-stderr-$$.txt | head -20 >&2
    rm -f /tmp/openapi-drift-build-stderr-$$.txt
    continue
  fi
  rm -f /tmp/openapi-drift-build-stderr-$$.txt

  BINARY="${BUILD_DIR}/debug/openapi_dump"
  if [[ ! -x "$BINARY" ]]; then
    log_fail "${dir}: binary not found at ${BINARY} after build"
    continue
  fi

  # Run openapi_dump and capture output
  FRESH_JSON="$(mktemp /tmp/openapi-fresh-$$.json)"
  if ! "${BINARY}" > "${FRESH_JSON}" 2>/tmp/openapi-dump-stderr-$$.txt; then
    log_fail "${dir}: openapi_dump exited non-zero"
    cat /tmp/openapi-dump-stderr-$$.txt | head -10 >&2
    rm -f "${FRESH_JSON}" /tmp/openapi-dump-stderr-$$.txt
    continue
  fi
  rm -f /tmp/openapi-dump-stderr-$$.txt

  # Normalise both and diff
  NORM_FRESH="$(mktemp /tmp/openapi-norm-fresh-$$.json)"
  NORM_COMMITTED="$(mktemp /tmp/openapi-norm-committed-$$.json)"
  normalize_json "${FRESH_JSON}" > "${NORM_FRESH}"
  normalize_json "${contract}"   > "${NORM_COMMITTED}"

  if diff --unified=3 "${NORM_COMMITTED}" "${NORM_FRESH}" > /tmp/openapi-diff-$$.patch 2>&1; then
    log_pass "${dir}: no drift (contract matches live code)"
  else
    log_fail "${dir}: DRIFT DETECTED — committed contract does not match openapi_dump output"
    echo ""
    echo "    Diff (committed ← → live code):"
    sed 's/^/    /' /tmp/openapi-diff-$$.patch | head -60
    echo ""
    DRIFT_MODULES+=("${dir}")
  fi

  rm -f "${FRESH_JSON}" "${NORM_FRESH}" "${NORM_COMMITTED}" /tmp/openapi-diff-$$.patch
  echo ""
done

# Warn-only modules
for mod in "${WARN_MODULES[@]}"; do
  if [[ -n "$SINGLE_MODULE" && "$SINGLE_MODULE" != "$mod" ]]; then
    continue
  fi
  if [[ -f "modules/${mod}/src/bin/openapi_dump.rs" ]]; then
    log_warn "${mod}: has openapi_dump but no committed contracts/${mod}/openapi.json — add a JSON contract to include in drift gate"
    echo ""
  fi
done

# ---------------------------------------------------------------------------
# Gate 2 — TypeScript client drift (optional)
# ---------------------------------------------------------------------------
if $CHECK_TS_CLIENTS; then
  echo "── Gate 2: TypeScript Client Drift ──────────────────────"
  echo ""

  if ! command -v npx &>/dev/null; then
    echo "  ⚠ npx not found — skipping TS client drift check (install Node.js to enable)"
    echo ""
  else
    TS_DRIFT=()

    for entry in "${ALL_MODULES[@]}"; do
      IFS='|' read -r dir _pkg _contract <<< "$entry"

      # Find committed .d.ts file
      DTS_FILE="$(find "clients/${dir}/src" -name "*.d.ts" -not -path "*/node_modules/*" 2>/dev/null | head -1)"
      if [[ -z "$DTS_FILE" ]]; then
        log_warn "${dir}: no committed .d.ts file in clients/${dir}/src/ — skipping TS check"
        continue
      fi

      # Generate fresh .d.ts from the fresh openapi.json already dumped
      # Use the contract file as input (avoids rebuilding)
      FRESH_DTS="$(mktemp /tmp/openapi-ts-fresh-$$.d.ts)"
      OPENAPI_SRC="${_contract:-contracts/${dir}/openapi.json}"

      if npx --prefix "clients/${dir}" openapi-typescript "${OPENAPI_SRC}" \
           --output "${FRESH_DTS}" --silent 2>/dev/null; then
        if diff --unified=3 "${DTS_FILE}" "${FRESH_DTS}" > /tmp/ts-diff-$$.patch 2>&1; then
          log_pass "${dir}: TS client types match committed .d.ts"
        else
          log_fail "${dir}: TS CLIENT DRIFT — committed .d.ts does not match generated types"
          echo "    Diff (committed ← → generated):"
          sed 's/^/    /' /tmp/ts-diff-$$.patch | head -40
          echo ""
          TS_DRIFT+=("${dir}")
        fi
        rm -f /tmp/ts-diff-$$.patch
      else
        log_warn "${dir}: openapi-typescript generation failed — skipping TS drift check"
      fi

      rm -f "${FRESH_DTS}"
      echo ""
    done

    if [[ ${#TS_DRIFT[@]} -gt 0 ]]; then
      FAIL=$((FAIL + ${#TS_DRIFT[@]}))
    fi
  fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo "── Summary ───────────────────────────────────────────────"
echo ""
echo "  Passed   : ${PASS}"
echo "  Warnings : ${WARN}"
echo "  Failed   : ${FAIL}"
echo ""

if [[ $FAIL -gt 0 ]]; then
  echo "  ❌ CONTRACT DRIFT DETECTED"
  if [[ ${#DRIFT_MODULES[@]} -gt 0 ]]; then
    echo ""
    echo "  Drifted modules:"
    for m in "${DRIFT_MODULES[@]}"; do
      echo "    • ${m} — regenerate: cargo run --bin openapi_dump -p <pkg> > contracts/${m}/openapi.json"
    done
  fi
  echo ""
  echo "  To fix: run the openapi_dump binary for each drifted module and commit the output."
  echo "  Then re-run this gate."
  echo ""
  exit 1
fi

echo "  ✅ No contract drift detected."
echo ""
exit 0
