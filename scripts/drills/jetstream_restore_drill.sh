#!/usr/bin/env bash
# JetStream Backup/Restore Drill
#
# Creates a temporary drill stream + consumer, snapshots it, deletes it,
# restores it, and verifies consumer delivery resumes.
#
# Usage:
#   bash scripts/drills/jetstream_restore_drill.sh
#
# Optional env:
#   NATS_SERVER      Default: nats://host.docker.internal:4222
#   NATS_HTTP_URL    Default: http://localhost:8222
#   NATS_BOX_IMAGE   Default: natsio/nats-box:latest
#   INITIAL_MESSAGES Default: 3
#
# Exit: 0 on PASS, non-zero on FAIL.

set -euo pipefail

NATS_SERVER="${NATS_SERVER:-nats://host.docker.internal:4222}"
NATS_HTTP_URL="${NATS_HTTP_URL:-http://localhost:8222}"
NATS_BOX_IMAGE="${NATS_BOX_IMAGE:-natsio/nats-box:latest}"
INITIAL_MESSAGES="${INITIAL_MESSAGES:-3}"

RUN_ID="$(date -u +%Y%m%d%H%M%S)"
STREAM="DRILL_JS_RESTORE_${RUN_ID}"
CONSUMER="DRILL_CONSUMER_${RUN_ID}"
SUBJECT="drill.jetstream.${RUN_ID}"
MARKER="resume-marker-${RUN_ID}"

WORK_DIR="$(mktemp -d -t jetstream-drill.XXXXXX)"
BACKUP_DIR="${WORK_DIR}/backup"
STREAM_INFO_BEFORE="${WORK_DIR}/stream_before.json"
STREAM_INFO_AFTER="${WORK_DIR}/stream_after.json"
CONSUMERS_AFTER="${WORK_DIR}/consumers_after.txt"
RESUME_PAYLOAD="${WORK_DIR}/resume_payload.txt"

PASS_COUNT=0
FAIL_COUNT=0

log() { echo "[jetstream-drill] $*"; }
pass() { echo "[jetstream-drill] PASS: $*"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { echo "[jetstream-drill] FAIL: $*"; FAIL_COUNT=$((FAIL_COUNT + 1)); }

run_nats() {
  docker run --rm \
    -v "${WORK_DIR}:/work" \
    "${NATS_BOX_IMAGE}" \
    nats -s "${NATS_SERVER}" "$@"
}

cleanup() {
  run_nats stream rm "${STREAM}" --force >/dev/null 2>&1 || true
  rm -rf "${WORK_DIR}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

log "JetStream restore drill start (run_id=${RUN_ID})"
log "NATS server: ${NATS_SERVER}"
log "NATS monitor: ${NATS_HTTP_URL}"
log "Working dir: ${WORK_DIR}"

curl -fsS "${NATS_HTTP_URL}/healthz" >/dev/null \
  && pass "NATS monitoring health endpoint is reachable" \
  || fail "NATS monitoring health endpoint is not reachable"

if [[ "${FAIL_COUNT}" -gt 0 ]]; then
  echo "RESULT: FAIL"
  exit 1
fi

run_nats stream ls >/dev/null
pass "JetStream API reachable via nats CLI"

run_nats stream add "${STREAM}" \
  --subjects "${SUBJECT}" \
  --storage file \
  --retention limits \
  --ack \
  --defaults >/dev/null
pass "Created drill stream ${STREAM}"

run_nats consumer add "${STREAM}" "${CONSUMER}" \
  --pull \
  --deliver all \
  --ack explicit \
  --defaults >/dev/null
pass "Created drill consumer ${CONSUMER}"

for i in $(seq 1 "${INITIAL_MESSAGES}"); do
  run_nats publish "${SUBJECT}" "drill-msg-${i}" >/dev/null
done
pass "Published ${INITIAL_MESSAGES} drill messages"

run_nats stream info "${STREAM}" --json >"${STREAM_INFO_BEFORE}"
messages_before="$(jq -r '.state.messages // 0' "${STREAM_INFO_BEFORE}")"
if [[ "${messages_before}" -ge "${INITIAL_MESSAGES}" ]]; then
  pass "Stream has expected message count before backup (${messages_before})"
else
  fail "Message count before backup too low (${messages_before})"
fi

mkdir -p "${BACKUP_DIR}"
run_nats stream backup "${STREAM}" /work/backup --check --consumers --no-progress >/dev/null
if [[ -f "${BACKUP_DIR}/backup.json" && -f "${BACKUP_DIR}/stream.tar.s2" ]]; then
  pass "Backup artifacts created (backup.json + stream.tar.s2)"
else
  fail "Backup artifacts missing in ${BACKUP_DIR}"
fi

run_nats stream rm "${STREAM}" --force >/dev/null
pass "Deleted drill stream before restore"

run_nats stream restore /work/backup --no-progress >/dev/null
pass "Restored stream from backup"

run_nats stream info "${STREAM}" --json >"${STREAM_INFO_AFTER}"
messages_after="$(jq -r '.state.messages // 0' "${STREAM_INFO_AFTER}")"
if [[ "${messages_after}" -ge "${messages_before}" ]]; then
  pass "Restored stream message count is valid (${messages_after})"
else
  fail "Restored stream lost messages (${messages_after} < ${messages_before})"
fi

run_nats consumer ls "${STREAM}" --names >"${CONSUMERS_AFTER}"
if grep -qx "${CONSUMER}" "${CONSUMERS_AFTER}"; then
  pass "Consumer ${CONSUMER} restored"
else
  fail "Consumer ${CONSUMER} missing after restore"
fi

pending="$(run_nats consumer info "${STREAM}" "${CONSUMER}" --json | jq -r '.num_pending // 0')"
if [[ "${pending}" -gt 0 ]]; then
  run_nats consumer next "${STREAM}" "${CONSUMER}" --count "${pending}" --ack >/dev/null
fi

run_nats publish "${SUBJECT}" "${MARKER}" >/dev/null
run_nats consumer next "${STREAM}" "${CONSUMER}" --count 1 --ack --raw >"${RESUME_PAYLOAD}"
if grep -q "${MARKER}" "${RESUME_PAYLOAD}"; then
  pass "Consumer resumed after restore and received new message"
else
  fail "Consumer did not receive post-restore marker message"
fi

echo ""
echo "Summary:"
echo "  checks_passed=${PASS_COUNT}"
echo "  checks_failed=${FAIL_COUNT}"
echo "  stream=${STREAM}"
echo "  consumer=${CONSUMER}"
echo "  backup_dir=${BACKUP_DIR}"

if [[ "${FAIL_COUNT}" -gt 0 ]]; then
  echo "RESULT: FAIL"
  exit 1
fi

echo "RESULT: PASS"
exit 0
