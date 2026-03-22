#!/usr/bin/env bash
# ============================================================
# seed-manufacturing.sh
# Seeds manufacturing demo data (numbering, GL, party, inventory, BOM, production)
# via the demo-seed tool. Complements seed-dev.sh which handles tenant
# provisioning, admin user, and AR seeding.
#
# Usage:
#   ./scripts/seed-manufacturing.sh --tenant dev-test-01
#   ./scripts/seed-manufacturing.sh --tenant dev-test-01 --seed 42
#   ./scripts/seed-manufacturing.sh --tenant dev-test-01 --manifest-out /tmp/manifest.json
#
# Run seed-dev.sh first to provision the tenant, then this script.
# ============================================================
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── Defaults ─────────────────────────────────────────────────
TENANT=""
SEED=42
MANIFEST_OUT=""
TOKEN="${DEMO_SEED_TOKEN:-}"

# Service URLs (match docker-compose defaults)
NUMBERING_URL="${NUMBERING_BASE_URL:-http://localhost:8120}"
GL_URL="${GL_BASE_URL:-http://localhost:8090}"
PARTY_URL="${PARTY_BASE_URL:-http://localhost:8098}"
INVENTORY_URL="${INVENTORY_BASE_URL:-http://localhost:8092}"
BOM_URL="${BOM_BASE_URL:-http://localhost:8107}"
PRODUCTION_URL="${PRODUCTION_BASE_URL:-http://localhost:8108}"

# ── Parse arguments ──────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tenant)       TENANT="$2"; shift 2 ;;
    --seed)         SEED="$2"; shift 2 ;;
    --manifest-out) MANIFEST_OUT="$2"; shift 2 ;;
    --token)        TOKEN="$2"; shift 2 ;;
    --help|-h)
      echo "Usage: $0 --tenant <TENANT_ID> [--seed N] [--manifest-out PATH] [--token JWT]"
      echo ""
      echo "Seeds manufacturing demo data into a provisioned tenant."
      echo "Run seed-dev.sh first to create the tenant and admin user."
      echo ""
      echo "Options:"
      echo "  --tenant        Tenant ID (required)"
      echo "  --seed          RNG seed for deterministic data (default: 42)"
      echo "  --manifest-out  Write JSON manifest of created IDs to file"
      echo "  --token         JWT bearer token for authenticated services (or set DEMO_SEED_TOKEN)"
      echo ""
      echo "Modules seeded: numbering, gl, party, inventory, bom, production"
      echo ""
      echo "Environment overrides:"
      echo "  NUMBERING_BASE_URL   (default: http://localhost:8120)"
      echo "  GL_BASE_URL          (default: http://localhost:8090)"
      echo "  PARTY_BASE_URL       (default: http://localhost:8098)"
      echo "  INVENTORY_BASE_URL   (default: http://localhost:8092)"
      echo "  BOM_BASE_URL         (default: http://localhost:8107)"
      echo "  PRODUCTION_BASE_URL  (default: http://localhost:8108)"
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$TENANT" ]]; then
  echo "ERROR: --tenant is required" >&2
  echo "Usage: $0 --tenant <TENANT_ID> [--seed N] [--manifest-out PATH]" >&2
  exit 1
fi

# ── Preflight: verify services are reachable ─────────────────
echo "=== Preflight: checking service health ==="
FAILED=0

health_checks=(
  "numbering:${NUMBERING_URL}/health"
  "gl:${GL_URL}/api/health"
  "party:${PARTY_URL}/api/health"
  "inventory:${INVENTORY_URL}/api/health"
  "bom:${BOM_URL}/api/health"
  "production:${PRODUCTION_URL}/api/health"
)

for entry in "${health_checks[@]}"; do
  svc="${entry%%:*}"
  url="${entry#*:}"
  if curl -sf --max-time 5 "$url" >/dev/null 2>&1; then
    echo "  ✓ $svc healthy"
  else
    echo "  ✗ $svc not reachable at $url" >&2
    FAILED=1
  fi
done

if [[ "$FAILED" -ne 0 ]]; then
  echo "" >&2
  echo "Some services are not reachable. Start the dev stack first:" >&2
  echo "  scripts/dev-watch.sh" >&2
  exit 1
fi
echo ""

# ── Build demo-seed if needed ────────────────────────────────
echo "=== Building demo-seed ==="
"$PROJECT_ROOT/scripts/cargo-slot.sh" build -p demo-seed --quiet 2>&1 || {
  echo "ERROR: Failed to build demo-seed" >&2
  exit 1
}

# Find the binary
BINARY=""
for slot in 1 2 3 4; do
  candidate="$PROJECT_ROOT/target-slot-${slot}/debug/demo-seed"
  if [[ -x "$candidate" ]]; then
    BINARY="$candidate"
    break
  fi
done
if [[ -z "$BINARY" ]]; then
  candidate="$PROJECT_ROOT/target/debug/demo-seed"
  if [[ -x "$candidate" ]]; then
    BINARY="$candidate"
  fi
fi
if [[ -z "$BINARY" ]]; then
  echo "ERROR: demo-seed binary not found after build" >&2
  exit 1
fi
echo "  Using: $BINARY"
echo ""

# ── Run demo-seed ────────────────────────────────────────────
echo "=== Seeding manufacturing data ==="
echo "  Tenant: $TENANT"
echo "  Seed:   $SEED"
echo ""

SEED_ARGS=(
  --tenant "$TENANT"
  --seed "$SEED"
  --modules "numbering,gl,party,inventory,bom,production"
  --numbering-url "$NUMBERING_URL"
  --gl-url "$GL_URL"
  --party-url "$PARTY_URL"
  --inventory-url "$INVENTORY_URL"
  --bom-url "$BOM_URL"
  --production-url "$PRODUCTION_URL"
)

if [[ -n "$MANIFEST_OUT" ]]; then
  SEED_ARGS+=(--manifest-out "$MANIFEST_OUT")
fi

if [[ -n "$TOKEN" ]]; then
  SEED_ARGS+=(--token "$TOKEN")
fi

OUTPUT=$("$BINARY" "${SEED_ARGS[@]}" 2>&1)
EXIT_CODE=$?

if [[ $EXIT_CODE -ne 0 ]]; then
  echo "ERROR: demo-seed failed (exit code $EXIT_CODE)" >&2
  echo "$OUTPUT" >&2
  exit 1
fi

# ── Summary ──────────────────────────────────────────────────
# Extract digest (64-char hex line)
DIGEST=$(echo "$OUTPUT" | grep -oE '^[a-f0-9]{64}$' | head -1)

echo "=== Manufacturing seed complete ==="
echo "  Tenant:  $TENANT"
echo "  Seed:    $SEED"
echo "  Digest:  ${DIGEST:-unknown}"
echo ""
echo "  Resources created:"
echo "    Numbering policies:  8"
echo "    GL accounts:         20"
echo "    GL FX rates:         2"
echo "    Customers:           5"
echo "    Suppliers:           5"
echo "    UoMs:                5"
echo "    Warehouse locations: 7"
echo "    Inventory items:     13"
echo "    BOMs:                5"
echo "    Work centers:        6"
echo "    Routings:            5"

if [[ -n "$MANIFEST_OUT" ]]; then
  echo ""
  echo "  Manifest: $MANIFEST_OUT"
fi

echo ""
echo "Done. Run seed-dev.sh first if you haven't already (tenant + admin + AR)."
