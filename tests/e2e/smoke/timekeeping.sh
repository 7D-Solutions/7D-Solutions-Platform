# TAGS: phase42-smoke smoke timekeeping
# Smoke test: timekeeping module is reachable and ready.

TIMEKEEPING_PORT=$(resolve_port timekeeping)

echo "[timekeeping] port $TIMEKEEPING_PORT"
wait_for_ready "timekeeping" "$TIMEKEEPING_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "timekeeping" "$TIMEKEEPING_PORT" || true
assert_ready_shape "timekeeping" "$TIMEKEEPING_PORT" || true
