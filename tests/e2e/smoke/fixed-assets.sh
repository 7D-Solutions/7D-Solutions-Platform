# TAGS: phase42-smoke smoke fixed-assets
# Smoke test: fixed-assets module is reachable and ready.

FIXED_ASSETS_PORT=$(resolve_port fixed-assets)

echo "[fixed-assets] port $FIXED_ASSETS_PORT"
wait_for_ready "fixed-assets" "$FIXED_ASSETS_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "fixed-assets" "$FIXED_ASSETS_PORT" || true
assert_ready_shape "fixed-assets" "$FIXED_ASSETS_PORT" || true
