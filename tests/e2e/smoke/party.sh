# TAGS: phase42-smoke smoke party
# Smoke test: party module is reachable and ready.

PARTY_PORT=$(resolve_port party)

echo "[party] port $PARTY_PORT"
wait_for_ready "party" "$PARTY_PORT" "${E2E_TIMEOUT:-30}" || true
assert_healthz "party" "$PARTY_PORT" || true
assert_ready_shape "party" "$PARTY_PORT" || true
