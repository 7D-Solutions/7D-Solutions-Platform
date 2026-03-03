#!/usr/bin/env bash
# DLQ Replay Drill — Orchestrator
#
# Runs the per-module DLQ replay drill binaries for the three highest
# operational-risk services (notifications, integrations, reporting).
# Each drill:
#   1. Injects a controlled failure to force a DLQ entry
#   2. Replays it through the real domain operation
#   3. Verifies status transitions and no duplicates
#
# Usage:
#   bash scripts/drills/dlq_replay_drill.sh
#
# Prerequisites:
#   - Docker Compose services running (databases reachable)
#   - cargo available on PATH
#
# Exit code: 0 if all drills pass, 1 if any fail.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Configuration ────────────────────────────────────────────────────

DRILLS=(
  "notifications|modules/notifications|Notifications DLQ replay (dead_lettered → pending → outbox event)"
  "integrations|modules/integrations|Integrations DLQ replay (failed_events → external_ref → outbox event)"
  "reporting|modules/reporting|Reporting DLQ replay (checkpoint reset → idempotent re-ingest)"
)

pass_count=0
fail_count=0
results=()

# ── Helper ───────────────────────────────────────────────────────────

run_drill() {
  local name="$1"
  local manifest_dir="$2"
  local description="$3"
  local manifest_path="$PROJECT_ROOT/$manifest_dir/Cargo.toml"

  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "DRILL: $description"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  local output
  local exit_code=0

  # Build and run via cargo-slot if available, fall back to cargo.
  if [[ -x "$PROJECT_ROOT/scripts/cargo-slot.sh" ]]; then
    output=$("$PROJECT_ROOT/scripts/cargo-slot.sh" run \
      --manifest-path "$manifest_path" \
      --bin dlq_replay_drill 2>&1) || exit_code=$?
  else
    output=$(cargo run \
      --manifest-path "$manifest_path" \
      --bin dlq_replay_drill 2>&1) || exit_code=$?
  fi

  echo "$output"

  # Check for the definitive success marker
  if [[ $exit_code -eq 0 ]] && echo "$output" | grep -q "dlq_replay_drill=ok"; then
    echo ""
    echo "  -> $name: PASS"
    results+=("PASS  $name — $description")
    pass_count=$((pass_count + 1))
  else
    echo ""
    echo "  -> $name: FAIL (exit_code=$exit_code)"
    results+=("FAIL  $name — $description")
    fail_count=$((fail_count + 1))
  fi
}

# ── Run drills ───────────────────────────────────────────────────────

echo "DLQ Replay Drill — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "Running ${#DRILLS[@]} drill(s) against live databases."

for entry in "${DRILLS[@]}"; do
  IFS='|' read -r name dir desc <<< "$entry"
  run_drill "$name" "$dir" "$desc"
done

# ── Summary ──────────────────────────────────────────────────────────

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "SUMMARY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
for r in "${results[@]}"; do
  echo "  $r"
done
echo ""
echo "Total: ${#DRILLS[@]}  Pass: $pass_count  Fail: $fail_count"

if [[ $fail_count -gt 0 ]]; then
  echo ""
  echo "RESULT: FAIL"
  exit 1
else
  echo ""
  echo "RESULT: PASS"
  exit 0
fi
