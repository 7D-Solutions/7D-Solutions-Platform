# Tenant Offboarding

This document describes the control-plane offboarding flow for tenants that are
leaving the platform.

## Export

The canonical user-facing export endpoint is:

```http
POST /api/control/tenants/{tenant_id}/export
```

The endpoint returns a deterministic ZIP bundle containing the control-plane
records required for portability and audit:

- `tenant.jsonl`
- `retention_policy.jsonl`
- `entitlements.jsonl`
- `provisioning_requests.jsonl`
- `manifest.json`

The bundle is deterministic:

- rows are ordered before serialisation,
- archive entry timestamps are fixed,
- entries are written in a stable order,
- the response includes a SHA-256 digest header for verification.

## Retention Timelines

The retention policy stored in `cp_retention_policies` controls the later
tombstone window:

- `export_ready_at` marks when the export bundle was produced.
- `auto_tombstone_days` defines the grace period before tombstoning is allowed.
- `data_retention_days` defines how long data must remain available after
  logical deletion before physical purge may proceed.

The recommended sequence is:

1. Export the tenant bundle.
2. Hand the bundle to the customer or compliance team.
3. Wait the configured grace period.
4. Tombstone the tenant using the GDPR erasure endpoint.
5. Allow downstream purge jobs to run after the retention window elapses.

## Operational Notes

- The export endpoint updates `cp_retention_policies.export_ready_at`.
- Existing tombstone aliases remain valid for internal workflows.
- Physical deletion is performed by downstream purge workers, not by the
  control-plane API itself.
