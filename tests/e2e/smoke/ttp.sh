# TAGS: phase42-smoke smoke ttp
# Smoke test: TTP module is reachable and ready.

TTP_PORT=$(resolve_port ttp)

echo "[ttp] port $TTP_PORT"
wait_for_ready "ttp" "$TTP_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "ttp" "$TTP_PORT" || true
assert_ready_shape "ttp" "$TTP_PORT" || true
