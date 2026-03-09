#!/usr/bin/env bash
# Cross-compile a Rust service on macOS and run it inside its Docker container.
#
# The binary is compiled natively (fast, incremental) targeting Linux aarch64,
# then volume-mounted into the container. No Docker compilation needed.
#
# Usage:
#   scripts/dev-cross.sh inventory            # Cross-compile + restart container (one-shot)
#   scripts/dev-cross.sh inventory --watch    # Watch mode: recompile + restart on file changes
#   scripts/dev-cross.sh --list               # Show available services
#
# Prerequisites:
#   - musl-cross installed: brew install filosottile/musl-cross/musl-cross
#   - Rust target: rustup target add aarch64-unknown-linux-musl
#   - cargo-watch installed (for --watch mode): cargo install cargo-watch
#   - .cargo/config.toml has [target.aarch64-unknown-linux-musl] linker set

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

TARGET="aarch64-unknown-linux-musl"
CROSS_DIR="target/$TARGET/debug"

# ── Service registry ──────────────────────────────────────────────
# Format: service|crate_name|container|service_port|bin_name
# Same services as dev-native.sh but only the fields needed for cross-compile.
SERVICES=(
  "ar|ar-rs|7d-ar|8086|ar-rs"
  "subscriptions|subscriptions-rs|7d-subscriptions|8087|subscriptions-rs"
  "payments|payments-rs|7d-payments|8088|payments-rs"
  "notifications|notifications-rs|7d-notifications|8089|notifications-rs"
  "gl|gl-rs|7d-gl|8090|gl-rs"
  "inventory|inventory-rs|7d-inventory|8092|inventory-rs"
  "ap|ap|7d-ap|8093|ap"
  "treasury|treasury|7d-treasury|8094|treasury"
  "fixed-assets|fixed-assets|7d-fixed-assets|8104|fixed-assets"
  "consolidation|consolidation|7d-consolidation|8105|consolidation"
  "timekeeping|timekeeping|7d-timekeeping|8097|timekeeping"
  "party|party-rs|7d-party|8098|party"
  "integrations|integrations-rs|7d-integrations|8099|integrations"
  "ttp|ttp-rs|7d-ttp|8100|ttp"
  "pdf-editor|pdf-editor-rs|7d-pdf-editor|8102|pdf-editor-rs"
  "maintenance|maintenance-rs|7d-maintenance|8101|maintenance-rs"
  "shipping-receiving|shipping-receiving-rs|7d-shipping-receiving|8103|shipping-receiving-rs"
  "quality-inspection|quality-inspection-rs|7d-quality-inspection|8106|quality-inspection-rs"
  "bom|bom-rs|7d-bom|8107|bom-rs"
  "production|production-rs|7d-production|8108|production-rs"
  "workflow|workflow|7d-workflow|8110|workflow"
  "numbering|numbering|7d-numbering|8120|numbering"
  "workforce-competence|workforce-competence-rs|7d-workforce-competence|8121|workforce-competence-rs"
  "customer-portal|customer-portal|7d-customer-portal|8111|customer-portal"
  "reporting|reporting|7d-reporting|8096|reporting"
  "control-plane|control-plane|7d-control-plane|8091|control-plane"
)

# ── Helpers ───────────────────────────────────────────────────────

list_services() {
  echo "Available services:"
  echo ""
  printf "  %-22s %-25s %-8s %s\n" "SERVICE" "CONTAINER" "PORT" "BINARY"
  printf "  %-22s %-25s %-8s %s\n" "-------" "---------" "----" "------"
  for entry in "${SERVICES[@]}"; do
    IFS='|' read -r svc _ container port bin <<< "$entry"
    printf "  %-22s %-25s %-8s %s\n" "$svc" "$container" "$port" "$bin"
  done
}

find_service() {
  local name="$1"
  for entry in "${SERVICES[@]}"; do
    IFS='|' read -r svc _ _ _ _ <<< "$entry"
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
  echo "Usage: scripts/dev-cross.sh <service> [--watch]"
  echo "       scripts/dev-cross.sh --list"
  exit 1
fi

SERVICE_NAME="$1"
MODE="${2:-once}"  # once (default) or --watch

ENTRY=$(find_service "$SERVICE_NAME") || {
  echo "Unknown service: $SERVICE_NAME"
  echo ""
  list_services
  exit 1
}

IFS='|' read -r SVC CRATE CONTAINER PORT BIN_NAME <<< "$ENTRY"

# ── Verify prerequisites ─────────────────────────────────────────

if ! command -v aarch64-linux-musl-gcc &>/dev/null; then
  echo "Error: musl-cross not found. Install with: brew install filosottile/musl-cross/musl-cross" >&2
  exit 1
fi

if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
  echo "Error: Rust target $TARGET not installed. Run: rustup target add $TARGET" >&2
  exit 1
fi

if [ "$MODE" = "--watch" ] && ! command -v cargo-watch &>/dev/null; then
  echo "Error: cargo-watch not found. Install with: cargo install cargo-watch" >&2
  exit 1
fi

# ── Cross-compile and restart ─────────────────────────────────────

cross_build_and_restart() {
  echo "Cross-compiling $SVC (crate: $CRATE, target: $TARGET)..."
  cargo build --target "$TARGET" -p "$CRATE" --bin "$BIN_NAME"

  echo "Restarting container $CONTAINER..."
  docker restart "$CONTAINER" 2>/dev/null || {
    echo "Warning: container $CONTAINER not running. Start it first." >&2
    return 1
  }

  # Wait for health check
  echo -n "Waiting for health..."
  for i in $(seq 1 15); do
    if curl -sf "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1; then
      echo " healthy!"
      curl -sf "http://127.0.0.1:${PORT}/api/health"
      echo ""
      return 0
    fi
    echo -n "."
    sleep 2
  done
  echo " timeout (service may still be starting)"
}

echo ""
echo "Cross-compile workflow for $SVC"
echo "  Binary: $CROSS_DIR/$BIN_NAME → $CONTAINER:/usr/local/bin/$BIN_NAME"
echo ""

if [ "$MODE" = "--watch" ]; then
  echo "Watching for changes (Ctrl+C to stop)..."
  echo ""
  cargo watch -s "cargo build --target $TARGET -p $CRATE --bin $BIN_NAME && docker restart $CONTAINER"
else
  cross_build_and_restart
fi
