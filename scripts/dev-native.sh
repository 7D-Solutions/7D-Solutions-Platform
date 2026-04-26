#!/usr/bin/env bash
# Run a Rust service natively with cargo-watch instead of in Docker.
#
# Usage:
#   scripts/dev-native.sh inventory        # Stop container, run natively with cargo watch
#   scripts/dev-native.sh inventory --run   # One-shot run (no file watching)
#   scripts/dev-native.sh inventory --stop  # Just stop the container (re-enable with: docker start 7d-inventory)
#   scripts/dev-native.sh --list            # Show available services
#
# When done developing, Ctrl+C and restart the container:
#   docker start 7d-inventory
#
# Prerequisites:
#   - Data stack running (docker compose -f docker-compose.data.yml up -d)
#   - cargo-watch installed (cargo install cargo-watch)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# ── Service registry ──────────────────────────────────────────────
# Format: service|crate_name|container|service_port|pg_host_port|pg_user|pg_pass|pg_db|bin_name
# crate_name = the -p argument to cargo.
# bin_name   = the --bin argument (when binary name differs from crate name).
# pg_host_port is the localhost port mapped in docker-compose.data.yml.
SERVICES=(
  "ar|ar-rs|7d-ar|8086|5434|ar_user|ar_pass|ar_db|ar-rs"
  "subscriptions|subscriptions-rs|7d-subscriptions|8087|5435|subscriptions_user|subscriptions_pass|subscriptions_db|subscriptions-rs"
  "payments|payments-rs|7d-payments|8088|5436|payments_user|payments_pass|payments_db|payments-rs"
  "notifications|notifications-rs|7d-notifications|8089|5437|notifications_user|notifications_pass|notifications_db|notifications-rs"
  "gl|gl-rs|7d-gl|8090|5438|gl_user|gl_pass|gl_db|gl-rs"
  "inventory|inventory-rs|7d-inventory|8092|5442|inventory_user|inventory_pass|inventory_db|inventory-rs"
  "ap|ap|7d-ap|8093|5443|ap_user|ap_pass|ap_db|ap"
  "treasury|treasury|7d-treasury|8094|5444|treasury_user|treasury_pass|treasury_db|treasury"
  "fixed-assets|fixed-assets|7d-fixed-assets|8104|5445|fixed_assets_user|fixed_assets_pass|fixed_assets_db|fixed-assets"
  "consolidation|consolidation|7d-consolidation|8105|5446|consolidation_user|consolidation_pass|consolidation_db|consolidation"
  "timekeeping|timekeeping|7d-timekeeping|8097|5447|timekeeping_user|timekeeping_pass|timekeeping_db|timekeeping"
  "party|party-rs|7d-party|8098|5448|party_user|party_pass|party_db|party"
  "integrations|integrations-rs|7d-integrations|8099|5449|integrations_user|integrations_pass|integrations_db|integrations"
  "ttp|ttp-rs|7d-ttp|8100|5451|ttp_user|ttp_pass|ttp_db|ttp"
  "pdf-editor|pdf-editor|7d-pdf-editor|8102|5453|pdf_editor_user|pdf_editor_pass|pdf_editor_db|pdf-editor"
  "maintenance|maintenance-rs|7d-maintenance|8101|5452|maintenance_user|maintenance_pass|maintenance_db|maintenance-rs"
  "shipping-receiving|shipping-receiving-rs|7d-shipping-receiving|8103|5454|shipping_receiving_user|shipping_receiving_pass|shipping_receiving_db|shipping-receiving-rs"
  "quality-inspection|quality-inspection-rs|7d-quality-inspection|8106|5459|quality_inspection_user|quality_inspection_pass|quality_inspection_db|quality-inspection-rs"
  "bom|bom-rs|7d-bom|8107|5450|bom_user|bom_pass|bom_db|bom-rs"
  "production|production-rs|7d-production|8108|5461|production_user|production_pass|production_db|production-rs"
  "workflow|workflow|7d-workflow|8110|5457|workflow_user|workflow_pass|workflow_db|workflow"
  "numbering|numbering|7d-numbering|8120|5456|numbering_user|numbering_pass|numbering_db|numbering"
  "workforce-competence|workforce-competence-rs|7d-workforce-competence|8121|5458|wc_user|wc_pass|workforce_competence_db|workforce-competence-rs"
  "customer-portal|customer-portal|7d-customer-portal|8111|5464|customer_portal_user|customer_portal_pass|customer_portal_db|customer-portal"
  "reporting|reporting|7d-reporting|8096|5463|reporting_user|reporting_pass|reporting_db|reporting"
  "control-plane|control-plane|7d-control-plane|8091|5441|tenant_registry_user|tenant_registry_pass|tenant_registry_db|control-plane"
  "auth|auth-rs|7d-auth-1|8080|5433|auth_user|auth_pass|auth_db|identity-auth"
)

# NATS connection for native processes (localhost, not Docker network)
NATS_TOKEN="${NATS_AUTH_TOKEN:-dev-nats-token}"
NATIVE_NATS_URL="nats://platform:${NATS_TOKEN}@127.0.0.1:4222"

# ── Helpers ───────────────────────────────────────────────────────

list_services() {
  echo "Available services:"
  echo ""
  printf "  %-22s %-18s %-8s %s\n" "SERVICE" "CONTAINER" "PORT" "CRATE"
  printf "  %-22s %-18s %-8s %s\n" "-------" "---------" "----" "-----"
  for entry in "${SERVICES[@]}"; do
    IFS='|' read -r svc crate container port _ _ _ _ <<< "$entry"
    printf "  %-22s %-18s %-8s %s\n" "$svc" "$container" "$port" "$crate"
  done
}

find_service() {
  local name="$1"
  for entry in "${SERVICES[@]}"; do
    IFS='|' read -r svc _ _ _ _ _ _ _ <<< "$entry"
    if [ "$svc" = "$name" ]; then
      echo "$entry"
      return 0
    fi
  done
  return 1
}

# ── Parse args ────────────────────────────────────────────────────

if [ "${1:-}" = "--list" ] || [ "${1:-}" = "-l" ]; then
  list_services
  exit 0
fi

if [ -z "${1:-}" ]; then
  echo "Usage: scripts/dev-native.sh <service> [--run|--stop]"
  echo "       scripts/dev-native.sh --list"
  exit 1
fi

SERVICE_NAME="$1"
MODE="${2:-watch}"  # watch (default), --run, or --stop

ENTRY=$(find_service "$SERVICE_NAME") || {
  echo "Unknown service: $SERVICE_NAME"
  echo ""
  list_services
  exit 1
}

IFS='|' read -r SVC CRATE CONTAINER PORT PG_PORT PG_USER PG_PASS PG_DB BIN_NAME <<< "$ENTRY"

# ── Verify prerequisites ─────────────────────────────────────────

if [ "$MODE" != "--stop" ]; then
  if ! command -v cargo-watch &>/dev/null && [ "$MODE" != "--run" ]; then
    echo "Error: cargo-watch not found. Install with: cargo install cargo-watch" >&2
    exit 1
  fi

  # Check that Postgres is reachable (if service has a DB)
  if [ "$PG_PORT" != "0" ]; then
    if ! nc -z 127.0.0.1 "$PG_PORT" 2>/dev/null; then
      echo "Error: Postgres for $SVC not reachable on 127.0.0.1:$PG_PORT" >&2
      echo "Is the data stack running?" >&2
      echo "  docker compose -f docker-compose.data.yml up -d" >&2
      exit 1
    fi
  fi

  # Check NATS is reachable
  if ! nc -z 127.0.0.1 4222 2>/dev/null; then
    echo "Warning: NATS not reachable on 127.0.0.1:4222" >&2
    echo "Some services may not start without NATS." >&2
  fi
fi

# ── Stop container ────────────────────────────────────────────────

echo "Stopping container $CONTAINER..."
docker stop "$CONTAINER" 2>/dev/null && echo "  Stopped." || echo "  Already stopped."

if [ "$MODE" = "--stop" ]; then
  echo ""
  echo "Container stopped. Restart with: docker start $CONTAINER"
  exit 0
fi

# ── Build env vars ────────────────────────────────────────────────

# DATABASE_URL rewritten to use localhost and the mapped port
if [ "$PG_PORT" != "0" ]; then
  NATIVE_DB_URL="postgres://${PG_USER}:${PG_PASS}@127.0.0.1:${PG_PORT}/${PG_DB}?sslmode=require"
else
  NATIVE_DB_URL=""
fi

export DATABASE_URL="$NATIVE_DB_URL"
export NATS_URL="$NATIVE_NATS_URL"
export BUS_TYPE="nats"
export HOST="127.0.0.1"
export PORT="$PORT"
export RUST_LOG="${RUST_LOG:-info}"

# Source .env for supplemental config (JWT keys, etc.), then re-apply
# our overrides so .env cannot accidentally clobber the rewritten URLs.
# (.env has container-internal hostnames; we need localhost.)
if [ -f "$PROJECT_ROOT/.env" ]; then
  _saved_db_url="$DATABASE_URL"
  _saved_nats_url="$NATS_URL"
  # shellcheck disable=SC1091
  set -a; source "$PROJECT_ROOT/.env" 2>/dev/null || true; set +a
  export DATABASE_URL="$_saved_db_url"
  export NATS_URL="$_saved_nats_url"
  if [ -n "${JWT_PUBLIC_KEY_PEM:-}" ]; then
    export JWT_PUBLIC_KEY="$JWT_PUBLIC_KEY_PEM"
  fi
fi

# ── Run ───────────────────────────────────────────────────────────

echo ""
echo "Running $SVC natively (crate: $CRATE, port: $PORT)"
echo "  DATABASE_URL = postgres://...@127.0.0.1:${PG_PORT}/${PG_DB}"
echo "  NATS_URL     = nats://...@127.0.0.1:4222"
echo "  RUST_LOG     = $RUST_LOG"
echo ""

trap 'echo ""; echo "Restarting container $CONTAINER..."; docker start "$CONTAINER" 2>/dev/null; echo "Done."' EXIT

if [ "$MODE" = "--run" ]; then
  echo "One-shot run (Ctrl+C to stop and restart container)..."
  echo ""
  cargo run -p "$CRATE" --bin "$BIN_NAME"
else
  echo "Watching for changes (Ctrl+C to stop and restart container)..."
  echo ""
  cargo watch -x "run -p $CRATE --bin $BIN_NAME"
fi
