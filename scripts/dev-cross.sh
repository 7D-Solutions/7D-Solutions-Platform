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

# Tracks containers restarted in the current invocation (used by stale-mount audit).
RESTARTED_THIS_CYCLE=()

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
  "auth|auth-rs|7d-auth-1,7d-auth-2|8080|identity-auth|/healthz"
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

IFS='|' read -r SVC CRATE CONTAINER PORT BIN_NAME HEALTH_PATH <<< "$ENTRY"
HEALTH_PATH="${HEALTH_PATH:-/api/health}"

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
  # Use cargo-slot.sh to avoid build lock contention with agents.
  # Raw cargo would write to target/ (the symlink), colliding with whoever holds that slot.
  "$PROJECT_ROOT/scripts/cargo-slot.sh" build --target "$TARGET" -p "$CRATE" --bin "$BIN_NAME"

  # Handle comma-separated container lists (e.g. auth-1,auth-2)
  IFS=',' read -ra _CONTAINERS <<< "$CONTAINER"
  for _c in "${_CONTAINERS[@]}"; do
    echo "Restarting container $_c..."
    if docker restart "$_c" 2>/dev/null; then
      RESTARTED_THIS_CYCLE+=("$_c")
    else
      echo "Warning: container $_c not running." >&2
    fi
  done

  # Wait for health check
  echo -n "Waiting for health..."
  for _i in $(seq 1 15); do
    if curl -sf "http://127.0.0.1:${PORT}${HEALTH_PATH}" >/dev/null 2>&1; then
      echo " healthy!"
      curl -sf "http://127.0.0.1:${PORT}${HEALTH_PATH}"
      echo ""
      return 0
    fi
    echo -n "."
    sleep 2
  done
  echo " timeout (service may still be starting)"
}

stale_mount_audit() {
  # Skip in CI environments — this is a dev-loop tool only.
  if [[ -n "${CI:-}" ]]; then
    return 0
  fi

  local log_file="$PROJECT_ROOT/logs/watcher-override.log"
  local max_stale=10
  local stale_count=0

  echo ""
  echo "Stale-mount audit..."

  local containers
  containers=$(docker ps --format '{{.Names}}' 2>/dev/null | grep '^7d-' || true)
  if [[ -z "$containers" ]]; then
    echo "  No running 7d-* containers, skipping."
    return 0
  fi

  while IFS= read -r ctr; do
    # Find bind-mount source for /app/service (empty if not bind-mounted).
    local host_path
    host_path=$(docker inspect "$ctr" \
      --format '{{range .Mounts}}{{if and (eq .Type "bind") (eq .Destination "/app/service")}}{{.Source}}{{end}}{{end}}' \
      2>/dev/null || true)

    [[ -z "$host_path" ]] && continue
    [[ ! -f "$host_path" ]] && {
      echo "  ANOMALY $ctr: bind-mount source not found on host: $host_path" >&2
      continue
    }

    # macOS host stat vs Linux container stat.
    local host_mtime ctr_mtime
    host_mtime=$(stat -f %m "$host_path" 2>/dev/null || echo "0")
    ctr_mtime=$(docker exec "$ctr" stat -c %Y /app/service 2>/dev/null || echo "0")

    if [[ "$host_mtime" == "0" || "$ctr_mtime" == "0" ]]; then
      echo "  SKIP $ctr: stat failed (host=$host_mtime ctr=$ctr_mtime)" >&2
      continue
    fi

    local delta=$(( host_mtime - ctr_mtime ))

    # Host older than container → build artifact issue, not a stale mount.
    if [[ $delta -lt 0 ]]; then
      echo "  ANOMALY $ctr: host binary older than container view (delta=${delta}s) — skipping" >&2
      echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ") ANOMALY ctr=$ctr host_mtime=$host_mtime ctr_mtime=$ctr_mtime delta=${delta}s source=stale-mount-detector" \
        >> "$log_file"
      continue
    fi

    # Within virtiofs lag tolerance.
    [[ $delta -le 2 ]] && continue

    stale_count=$(( stale_count + 1 ))
    if [[ $stale_count -gt $max_stale ]]; then
      echo "  WARNING: $stale_count stale containers exceed threshold ($max_stale) — stopping audit. Investigate bind-mount configuration." >&2
      echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ") WARNING stale_count=$stale_count threshold=$max_stale source=stale-mount-detector" \
        >> "$log_file"
      return 0
    fi

    # Double-restart guard: this container should not have been restarted already.
    local already=0
    for r in "${RESTARTED_THIS_CYCLE[@]+"${RESTARTED_THIS_CYCLE[@]}"}"; do
      [[ "$r" == "$ctr" ]] && { already=1; break; }
    done
    if [[ $already -eq 1 ]]; then
      echo "  ERROR: $ctr would be restarted twice in this cycle — stopping. Check cargo-slot target dir for bind-mount permission issues." >&2
      echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ") ERROR double-restart-guard ctr=$ctr source=stale-mount-detector" \
        >> "$log_file"
      return 1
    fi

    echo "  Stale: $ctr (host=$host_mtime ctr=$ctr_mtime delta=${delta}s) — restarting..."
    if AGENTCORE_WATCHER_OVERRIDE=1 docker restart "$ctr" 2>/dev/null; then
      RESTARTED_THIS_CYCLE+=("$ctr")
      echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ") RESTART ctr=$ctr host_mtime=$host_mtime ctr_mtime=$ctr_mtime delta=${delta}s source=stale-mount-detector" \
        >> "$log_file"
    else
      echo "  WARNING: failed to restart $ctr" >&2
    fi
  done <<< "$containers"

  if [[ $stale_count -eq 0 ]]; then
    echo "  All containers in sync."
  else
    echo "  Stale-mount audit complete: $stale_count container(s) restarted."
  fi
}

echo ""
echo "Cross-compile workflow for $SVC"
echo "  Binary: $CROSS_DIR/$BIN_NAME → $CONTAINER:/usr/local/bin/$BIN_NAME"
echo ""

if [ "$MODE" = "--watch" ]; then
  echo "Watching for changes (Ctrl+C to stop)..."
  echo ""
  cargo watch -s "$PROJECT_ROOT/scripts/cargo-slot.sh build --target $TARGET -p $CRATE --bin $BIN_NAME && for c in ${CONTAINER//,/ }; do docker restart \$c; done"
else
  cross_build_and_restart
  stale_mount_audit
fi
