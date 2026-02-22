# Projection Rebuild Runbook

**Phase 48 — Production Hardening (last updated: bd-3len)**

## Purpose

Procedures for rebuilding read-model projections when they fall behind,
become inconsistent, or need to be reset after a DR event.

Two tools are available:
- **`projection-rebuild` CLI** — standalone binary for safe blue/green rebuild
- **Admin HTTP endpoints** — per-module endpoints for status checks and consistency verification

---

## When to Rebuild

| Trigger | Action |
|---------|--------|
| Projection lag > 60 s detected by monitoring | Check status, then rebuild if confirmed stale |
| After a DR restore | Rebuild all projections from restored event sources |
| Schema migration changes projection shape | Rebuild affected projections |
| Consistency check reports mismatches | Rebuild that projection |
| Manual support request for stale data | Check status first; rebuild if warranted |

---

## 1. projection-rebuild CLI

The `projection-rebuild` binary lives at `tools/projection-rebuild/`.

### Build

```bash
# Use the cargo slot shim (required — never call cargo directly)
./scripts/cargo-slot.sh build -p projection-rebuild
```

### Check projection status

```bash
PROJECTION_REBUILD_ROLE=operator PROJECTION_REBUILD_ACTOR="$(whoami)" \
  ./target/debug/projection-rebuild status <projection-name>

# List all known projections
PROJECTION_REBUILD_ROLE=operator PROJECTION_REBUILD_ACTOR="$(whoami)" \
  ./target/debug/projection-rebuild list
```

### Rebuild a specific projection (blue/green swap)

The rebuild uses a blue/green strategy: it writes to a shadow slot and swaps
atomically, so the live projection is never unavailable during rebuild.

```bash
PROJECTION_REBUILD_ROLE=admin PROJECTION_REBUILD_ACTOR="$(whoami)" \
  ./target/debug/projection-rebuild rebuild <projection-name>
```

### Verify projection integrity

Computes a digest of the projection state and compares against source events:

```bash
PROJECTION_REBUILD_ROLE=auditor PROJECTION_REBUILD_ACTOR="$(whoami)" \
  ./target/debug/projection-rebuild verify <projection-name>
```

### Required roles

| Command | Minimum role |
|---------|-------------|
| `list` | `auditor` |
| `status` | `operator` |
| `verify` | `auditor` |
| `rebuild` | `admin` |

Set via `--role` flag or `PROJECTION_REBUILD_ROLE` environment variable.

---

## 2. Admin HTTP Endpoints

Each module exposes standardized admin endpoints. All require the
`X-Admin-Token` header.

### Admin token

```bash
export ADMIN_TOKEN="${ADMIN_TOKEN:-$(grep ADMIN_TOKEN .env 2>/dev/null | cut -d= -f2)}"
```

### Endpoint reference

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/{module}/admin/projections` | GET | List projections with lag info |
| `/api/{module}/admin/projection-status` | POST | Status for a specific projection |
| `/api/{module}/admin/consistency-check` | POST | Verify projection vs source of truth |

### Module base URLs

| Module | Port | Example base URL |
|--------|------|-----------------|
| ar | 8086 | `http://localhost:8086` |
| subscriptions | 8087 | `http://localhost:8087` |
| payments | 8088 | `http://localhost:8088` |
| notifications | 8089 | `http://localhost:8089` |
| gl | 8090 | `http://localhost:8090` |
| inventory | 8092 | `http://localhost:8092` |
| ap | 8093 | `http://localhost:8093` |
| treasury | 8094 | `http://localhost:8094` |
| fixed-assets | 8095 | `http://localhost:8095` |
| consolidation | 8096 | `http://localhost:8096` |
| timekeeping | 8097 | `http://localhost:8097` |
| party | 8098 | `http://localhost:8098` |
| integrations | 8099 | `http://localhost:8099` |
| ttp | 8100 | `http://localhost:8100` |

### List projections (all modules sweep)

```bash
ADMIN_TOKEN="your-token-here"

for svc_port in \
  "ar:8086" "subscriptions:8087" "payments:8088" "notifications:8089" \
  "gl:8090" "inventory:8092" "ap:8093" "treasury:8094" \
  "fixed-assets:8095" "consolidation:8096" "timekeeping:8097" \
  "party:8098" "integrations:8099" "ttp:8100"; do
  svc="${svc_port%%:*}"
  port="${svc_port##*:}"
  echo "=== ${svc} ==="
  curl -sf -H "X-Admin-Token: ${ADMIN_TOKEN}" \
    "http://localhost:${port}/api/${svc}/admin/projections" | jq '.'
done
```

### Check projection status (specific)

```bash
curl -sf -X POST \
  -H "X-Admin-Token: ${ADMIN_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"projection_name": "ar_invoice_summary"}' \
  http://localhost:8086/api/ar/admin/projection-status | jq '.'
```

### Run consistency check

```bash
curl -sf -X POST \
  -H "X-Admin-Token: ${ADMIN_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"projection_name": "ar_invoice_summary"}' \
  http://localhost:8086/api/ar/admin/consistency-check | jq '.'
```

A successful consistency check returns `"consistent": true`. If false, rebuild.

---

## 3. Full-Platform Rebuild (After DR)

After a DR restore, rebuild projections in dependency order:

```bash
PROJECTION_REBUILD_ROLE=admin PROJECTION_REBUILD_ACTOR=dr-recovery

# 1. Platform-tier projections first
for proj in tenant_summary audit_trail; do
  ./target/debug/projection-rebuild rebuild "${proj}" \
    --role admin --actor dr-recovery
done

# 2. Financial projections
for proj in gl_trial_balance ar_aging ap_aging treasury_position; do
  ./target/debug/projection-rebuild rebuild "${proj}" \
    --role admin --actor dr-recovery
done

# 3. Operational projections
for proj in inventory_snapshot subscription_status timekeeping_summary; do
  ./target/debug/projection-rebuild rebuild "${proj}" \
    --role admin --actor dr-recovery
done

# 4. Verify all
./target/debug/projection-rebuild list --role auditor --actor dr-recovery
```

---

## 4. Lag Monitoring

Check event bus consumer lag to detect projection drift:

```bash
# NATS consumer lag per stream
nats consumer list PLATFORM --server localhost:4222

# Lag for a specific consumer
nats consumer info PLATFORM gl.posting.requested.consumer --server localhost:4222
```

Alert thresholds (from `docs/ops/ALERT-THRESHOLDS.md`):
- Warning: lag > 30 s
- Critical: lag > 300 s (5 min)

---

## Troubleshooting

### Rebuild fails with authorization error

```
Error: insufficient role for rebuild operation
```

Ensure `PROJECTION_REBUILD_ROLE=admin`. Non-admin roles cannot trigger rebuilds.

### Consistency check returns mismatches but no rebuild needed

Row count mismatches after a recent write are expected (eventual consistency).
Wait 30 s and re-run. If still mismatched, proceed with rebuild.

### Blue/green swap leaves old slot

If rebuild is interrupted mid-swap:
```bash
# Check swap state
./target/debug/projection-rebuild status <projection-name> --role operator --actor ops

# Re-run rebuild to complete the swap
./target/debug/projection-rebuild rebuild <projection-name> --role admin --actor ops
```

---

## References

- `tools/projection-rebuild/src/main.rs` — CLI source
- `tools/projection-rebuild/src/swap.rs` — blue/green swap logic
- `docs/runbooks/disaster_recovery.md` — when to use this after DR
- `docs/runbooks/incident_response.md` — lag alert response
- `docs/ops/ALERT-THRESHOLDS.md` — lag alert thresholds

## Changelog

- **2026-02-22**: Phase 48 — update header (bd-3len)
- **2026-02-19**: Phase 34 — initial projection rebuild runbook (bd-x48w)
