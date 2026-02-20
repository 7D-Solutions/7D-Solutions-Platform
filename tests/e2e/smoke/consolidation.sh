# TAGS: phase42-smoke smoke consolidation
# Smoke test: consolidation module is reachable and ready.

CONSOLIDATION_PORT=$(resolve_port consolidation)

echo "[consolidation] port $CONSOLIDATION_PORT"
wait_for_ready "consolidation" "$CONSOLIDATION_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "consolidation" "$CONSOLIDATION_PORT" || true
assert_ready_shape "consolidation" "$CONSOLIDATION_PORT" || true
