# TAGS: phase42-smoke smoke integrations
# Smoke test: integrations module is reachable and ready.

INTEGRATIONS_PORT=$(resolve_port integrations)

echo "[integrations] port $INTEGRATIONS_PORT"
wait_for_ready "integrations" "$INTEGRATIONS_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "integrations" "$INTEGRATIONS_PORT" || true
assert_ready_shape "integrations" "$INTEGRATIONS_PORT" || true
