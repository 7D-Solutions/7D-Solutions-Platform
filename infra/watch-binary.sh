#!/bin/bash
# Generic binary watcher for 7D platform services.
# Reads SERVICE_BINARY env var to know which binary to watch.
#
# Three responsibilities:
#   1. Detect new binaries (checksum poll) and restart the service
#   2. Validate binaries before loading (ELF header + stability check)
#   3. Self-heal: if the service is dead, periodically try to revive it

BINARY="${SERVICE_BINARY:?SERVICE_BINARY env var must be set}"
POLL_INTERVAL=3
STABLE_CHECKS=2          # consecutive identical checksums before restart
HEALTH_CHECK_INTERVAL=10 # polls between health checks (30s at 3s poll)
SUPERVISOR="supervisorctl -s unix:///tmp/supervisor.sock"

get_checksum() {
  md5sum "$BINARY" 2>/dev/null | awk '{print $1}'
}

get_size() {
  stat -c %s "$BINARY" 2>/dev/null || stat -f %z "$BINARY" 2>/dev/null
}

is_valid_elf() {
  # Check first 4 bytes are the ELF magic number: 0x7f ELF
  local magic
  magic=$(head -c 4 "$BINARY" 2>/dev/null | od -A n -t x1 | tr -d ' ')
  [ "$magic" = "7f454c46" ]
}

service_status() {
  $SUPERVISOR status service 2>/dev/null | awk '{print $2}'
}

restart_service() {
  local reason="$1"
  echo "[watcher] $reason"

  # Validate binary before starting
  if ! is_valid_elf; then
    echo "[watcher] ERROR: $BINARY is not a valid ELF binary ($(get_size) bytes) — skipping restart"
    return 1
  fi

  echo "[watcher] Binary is valid ELF ($(get_size) bytes) — starting service..."
  # stop+start (not restart) to recover from FATAL state
  $SUPERVISOR stop service 2>/dev/null
  if $SUPERVISOR start service 2>/dev/null; then
    # Verify it actually stayed up
    sleep 4
    local st
    st=$(service_status)
    if [ "$st" = "RUNNING" ]; then
      echo "[watcher] Service is RUNNING"
      return 0
    else
      echo "[watcher] WARNING: service started but status is $st"
      return 1
    fi
  else
    echo "[watcher] ERROR: supervisorctl start failed"
    return 1
  fi
}

wait_for_stable_checksum() {
  local current="$1"
  local stable_count=0

  while [ "$stable_count" -lt "$STABLE_CHECKS" ]; do
    sleep "$POLL_INTERVAL"
    local new_sum
    new_sum=$(get_checksum)
    if [ "$new_sum" = "$current" ]; then
      stable_count=$((stable_count + 1))
    else
      echo "[watcher] Binary still being written (${current:0:8} -> ${new_sum:0:8})..."
      current="$new_sum"
      stable_count=0
    fi
  done

  echo "$current"
}

# --- Main loop ---

LAST_SUM=$(get_checksum)
echo "[watcher] Watching $BINARY (initial checksum: ${LAST_SUM:0:8}...)"

poll_count=0

while true; do
  sleep "$POLL_INTERVAL"
  poll_count=$((poll_count + 1))

  # --- Binary change detection ---
  CURRENT_SUM=$(get_checksum)
  if [ "$CURRENT_SUM" != "$LAST_SUM" ] && [ -n "$CURRENT_SUM" ]; then
    echo "[watcher] Binary change detected (${LAST_SUM:0:8} -> ${CURRENT_SUM:0:8}) — waiting for write to finish..."
    CURRENT_SUM=$(wait_for_stable_checksum "$CURRENT_SUM")
    restart_service "Binary stable (${CURRENT_SUM:0:8}) — restarting service..."
    LAST_SUM="$CURRENT_SUM"
    poll_count=0
    continue
  fi

  # --- Self-heal: check service health periodically ---
  if [ $((poll_count % HEALTH_CHECK_INTERVAL)) -eq 0 ]; then
    st=$(service_status)
    if [ "$st" != "RUNNING" ]; then
      echo "[watcher] Service is $st — attempting recovery..."
      restart_service "Self-heal: service was $st"
    fi
  fi
done
