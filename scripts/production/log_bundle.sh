#!/usr/bin/env bash
# log_bundle.sh — Capture a timestamped diagnostic log bundle for the 7D Platform.
#
# Collects docker logs from all platform services (or a named subset) into a
# single tar.gz archive. No environment files, secrets, or credentials are
# included in the bundle.
#
# Usage:
#   bash scripts/production/log_bundle.sh [OPTIONS]
#
# Options:
#   --since WINDOW      Log window to capture.  Accepts Docker duration strings
#                       (e.g. 1h, 30m, 2h) or ISO 8601 timestamps.
#                       Default: 1h
#   --until TIMESTAMP   End of time window (ISO 8601).  Default: now.
#   --services LIST     Comma-separated container names to include.
#                       Default: all platform services (see ALL_SERVICES below).
#   --out DIR           Output directory for the bundle.  Default: /tmp
#   --dry-run           Print what would be collected; do not capture.
#
# Output:
#   /tmp/7d-log-bundle-YYYYMMDD-HHMMSS.tar.gz   (by default)
#
# The bundle contains:
#   manifest.txt            — capture metadata (timestamp, window, host)
#   container-list.txt      — `docker ps` output at capture time
#   logs/<service>.log      — stdout+stderr log lines for each service
#
# The bundle does NOT contain:
#   - Environment files (.env, secrets.env)
#   - Any file from /etc/7d/
#   - docker inspect output (may contain env vars with secrets)
#   - Any output of `env` or `printenv`
#
# Exit code: 0 = bundle created successfully.  Non-zero = fatal error.

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
SINCE="${SINCE:-1h}"
UNTIL_ARG=""          # empty = now (omit --until from docker logs)
OUT_DIR="${OUT_DIR:-/tmp}"
DRY_RUN=false
CUSTOM_SERVICES=""

# All platform containers in dependency order (infrastructure first)
ALL_SERVICES=(
    # Infrastructure
    "7d-nats"
    "7d-prometheus"
    "7d-grafana"

    # Databases (rarely need logs but capture on request)
    "7d-auth-postgres"
    "7d-ar-postgres"
    "7d-subscriptions-postgres"
    "7d-payments-postgres"
    "7d-notifications-postgres"
    "7d-gl-postgres"
    "7d-audit-postgres"
    "7d-tenant-registry-postgres"
    "7d-inventory-postgres"
    "7d-ap-postgres"
    "7d-treasury-postgres"
    "7d-fixed-assets-postgres"
    "7d-consolidation-postgres"
    "7d-timekeeping-postgres"
    "7d-party-postgres"
    "7d-integrations-postgres"
    "7d-ttp-postgres"
    "7d-projections-postgres"

    # Platform services
    "7d-auth-1"
    "7d-auth-2"
    "7d-auth-lb"
    "7d-control-plane"

    # Billing spine (highest priority)
    "7d-ttp"
    "7d-payments"
    "7d-ar"
    "7d-subscriptions"

    # Supporting modules
    "7d-gl"
    "7d-notifications"
    "7d-inventory"
    "7d-ap"
    "7d-treasury"
    "7d-fixed-assets"
    "7d-consolidation"
    "7d-timekeeping"
    "7d-party"
    "7d-integrations"

    # Frontend
    "7d-tcp-ui"
)

# ── Argument parsing ───────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --since)    SINCE="$2";           shift 2 ;;
        --until)    UNTIL_ARG="$2";       shift 2 ;;
        --services) CUSTOM_SERVICES="$2"; shift 2 ;;
        --out)      OUT_DIR="$2";         shift 2 ;;
        --dry-run)  DRY_RUN=true;         shift   ;;
        -h|--help)
            sed -n '2,/^# Exit/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *)
            echo "[log_bundle] ERROR: Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# ── Resolve service list ───────────────────────────────────────────────────────
if [[ -n "$CUSTOM_SERVICES" ]]; then
    IFS=',' read -ra SERVICES <<< "$CUSTOM_SERVICES"
else
    SERVICES=("${ALL_SERVICES[@]}")
fi

# ── Build docker logs flags ────────────────────────────────────────────────────
LOG_FLAGS=(--timestamps --since "$SINCE")
if [[ -n "$UNTIL_ARG" ]]; then
    LOG_FLAGS+=(--until "$UNTIL_ARG")
fi

# ── Dry-run mode ───────────────────────────────────────────────────────────────
if [[ "$DRY_RUN" == "true" ]]; then
    echo "[log_bundle] DRY-RUN — would capture:"
    echo "  Window : since=${SINCE}${UNTIL_ARG:+, until=${UNTIL_ARG}}"
    echo "  Services:"
    for svc in "${SERVICES[@]}"; do
        echo "    $svc"
    done
    echo "  Output : ${OUT_DIR}/7d-log-bundle-<timestamp>.tar.gz"
    exit 0
fi

# ── Staging area ──────────────────────────────────────────────────────────────
TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
STAGING="$(mktemp -d /tmp/7d-log-bundle-XXXXXX)"
LOG_DIR="${STAGING}/logs"
mkdir -p "$LOG_DIR"

log()  { echo "[log_bundle] $*"; }
warn() { echo "[log_bundle] WARN: $*" >&2; }

# ── manifest.txt ──────────────────────────────────────────────────────────────
cat > "${STAGING}/manifest.txt" <<MANIFEST
7D Platform — Diagnostic Log Bundle
Generated : $(date -u +"%Y-%m-%dT%H:%M:%SZ")
Host      : $(hostname -f 2>/dev/null || hostname)
Window    : since=${SINCE}${UNTIL_ARG:+, until=${UNTIL_ARG}}
Services  : ${#SERVICES[@]} requested
MANIFEST

# ── container-list.txt — docker ps output (no env, no secrets) ────────────────
log "Capturing container list..."
docker ps --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.RunningFor}}' \
    > "${STAGING}/container-list.txt" 2>&1 || warn "docker ps failed"

# ── Collect logs per service ──────────────────────────────────────────────────
CAPTURED=0
SKIPPED=0

log "Collecting logs (since=${SINCE}${UNTIL_ARG:+, until=${UNTIL_ARG}})..."

for svc in "${SERVICES[@]}"; do
    # Skip containers that are not running — avoids error noise
    if ! docker ps --format '{{.Names}}' | grep -qx "$svc"; then
        warn "$svc not running — skipping"
        echo "SKIPPED (not running)" > "${LOG_DIR}/${svc}.log"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    log "  $svc"
    # Capture stdout + stderr; docker logs always writes to stderr on older engines
    # Explicitly redirect both to prevent secret leakage via error messages
    if docker logs "${LOG_FLAGS[@]}" "$svc" > "${LOG_DIR}/${svc}.log" 2>&1; then
        CAPTURED=$((CAPTURED + 1))
    else
        warn "$svc — docker logs returned non-zero (partial capture may exist)"
        CAPTURED=$((CAPTURED + 1))
    fi
done

# ── NATS monitoring snapshot (no credentials, public endpoint) ─────────────────
if docker ps --format '{{.Names}}' | grep -qx "7d-nats"; then
    log "Capturing NATS monitoring snapshot..."
    {
        echo "=== /varz (server variables) ==="
        curl -sf http://localhost:8222/varz 2>/dev/null || echo "unavailable"
        echo ""
        echo "=== /jsz (JetStream summary) ==="
        curl -sf "http://localhost:8222/jsz" 2>/dev/null || echo "unavailable"
        echo ""
        echo "=== /connz (connection count) ==="
        curl -sf "http://localhost:8222/connz?subs=0" 2>/dev/null || echo "unavailable"
    } > "${STAGING}/nats-monitoring.json" 2>&1 || warn "NATS monitoring snapshot failed"
fi

# ── Finalize manifest ─────────────────────────────────────────────────────────
cat >> "${STAGING}/manifest.txt" <<MANIFEST
Captured  : ${CAPTURED}
Skipped   : ${SKIPPED}
MANIFEST

# ── Create archive ────────────────────────────────────────────────────────────
BUNDLE="${OUT_DIR}/7d-log-bundle-${TIMESTAMP}.tar.gz"

log "Creating bundle: $BUNDLE"
tar -czf "$BUNDLE" -C "$(dirname "$STAGING")" "$(basename "$STAGING")"

# ── Cleanup staging area ──────────────────────────────────────────────────────
rm -rf "$STAGING"

# ── Summary ───────────────────────────────────────────────────────────────────
BUNDLE_SIZE="$(du -sh "$BUNDLE" | cut -f1)"
log "Done."
echo ""
echo "  Bundle : $BUNDLE"
echo "  Size   : ${BUNDLE_SIZE}"
echo "  Window : since=${SINCE}${UNTIL_ARG:+, until=${UNTIL_ARG}}"
echo "  Logs   : ${CAPTURED} captured, ${SKIPPED} skipped (not running)"
echo ""
echo "  Transfer to local machine:"
echo "    scp deploy@prod.7dsolutions.example.com:${BUNDLE} ."
echo ""
echo "  Inspect:"
echo "    tar -tzf $(basename "$BUNDLE")   # list contents"
echo "    tar -xzf $(basename "$BUNDLE")   # extract"
