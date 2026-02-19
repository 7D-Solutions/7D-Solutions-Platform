# Data Retention & Export/Delete Framework

**Phase 34 – Hardening / Launch Readiness**

---

## Overview

Every tenant has a *retention policy* stored in `cp_retention_policies`. The
policy governs three lifecycle stages after a tenant is deleted:

1. **Export** – produce a deterministic, verifiable JSONL artifact.
2. **Tombstone window** – grace period between export and permitted purge.
3. **Tombstone** – soft-purge marker; downstream jobs may physically delete
   data once `data_retention_days` from `deleted_at` has elapsed.

Physical deletion is always performed by a separate scheduled job (not this
framework). The framework only sets markers and audits them.

---

## Retention Config (`cp_retention_policies`)

| Column | Default | Meaning |
|---|---|---|
| `data_retention_days` | 2555 (~7 yrs) | Days data must be retained after `deleted_at` |
| `export_format` | `jsonl` | Export artifact format |
| `auto_tombstone_days` | 30 | Days between `export_ready_at` and permitted tombstone |
| `export_ready_at` | NULL | Set when first export artifact is produced |
| `data_tombstoned_at` | NULL | Set when tombstone operation runs |

### Reading/updating via API

```
# Read current config (returns defaults if no row exists yet)
GET /api/control/tenants/{tenant_id}/retention

# Upsert config
PUT /api/control/tenants/{tenant_id}/retention
Content-Type: application/json
{"data_retention_days": 1095, "auto_tombstone_days": 14}
```

---

## Export Job

Export produces a deterministic JSONL artifact containing all tenant metadata
held in the platform registry. **Same database state → same bytes → same
SHA-256 digest.**

Rows included (sorted within each type for determinism):

| Record type | Source table |
|---|---|
| `tenant` | `tenants` |
| `retention_policy` | `cp_retention_policies` |
| `entitlement` | `cp_entitlements` |
| `provisioning_request` | `provisioning_requests` (idempotency keys redacted) |

### Running an export

```bash
# Requires TENANT_REGISTRY_DATABASE_URL
export TENANT_REGISTRY_DATABASE_URL=postgres://...

tenantctl tenant export --tenant <uuid-or-name> --output /tmp/tenant-export.jsonl
# → prints SHA-256 digest; updates export_ready_at
```

The export command also updates `export_ready_at` in `cp_retention_policies`,
starting the tombstone grace window.

---

## Tombstone Path

Tombstoning marks a tenant's data as ready for physical purge. It:

1. **Requires** the tenant to be in `deleted` state.
2. Is **idempotent** — calling it twice returns the existing timestamp.
3. Sets `cp_retention_policies.data_tombstoned_at`.
4. Writes a `tenant.data_tombstoned` event to `provisioning_outbox`.

```bash
# Via API
POST /api/control/tenants/{tenant_id}/tombstone

# Via CLI (not yet implemented — use API)
```

### Audit trail

Every tombstone operation writes to the provisioning outbox:

```json
{
  "event_type": "tenant.data_tombstoned",
  "tenant_id": "...",
  "data_tombstoned_at": "2026-02-20T...",
  "occurred_at": "2026-02-20T..."
}
```

The outbox relay forwards this event to NATS for downstream consumers (e.g.
physical purge jobs, compliance reporting).

---

## Recommended Sequence

```
1. Tenant lifecycle: active → suspended → deleted
   (tenantctl tenant suspend / deprovision)

2. Export tenant data:
   tenantctl tenant export --tenant <id> --output /path/export.jsonl

3. Verify digest matches expected value (if known):
   sha256sum /path/export.jsonl

4. After auto_tombstone_days grace period:
   POST /api/control/tenants/<id>/tombstone

5. Physical purge job runs when:
   NOW() > deleted_at + data_retention_days
```

---

## Defaults Rationale

| Parameter | Value | Rationale |
|---|---|---|
| `data_retention_days` | 2555 | GDPR / SOC 2 typical 7-year minimum |
| `auto_tombstone_days` | 30 | Gives data subjects 30 days to review export |
| `export_format` | `jsonl` | Machine-readable, streamable, easy to diff |

These defaults may be overridden per tenant via the API.
