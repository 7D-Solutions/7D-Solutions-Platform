# Soft Delete Standard

This document defines the platform-wide convention for tenant data that is
retained after deletion but no longer considered active.

## Core Rules

1. Every module that retains tenant-scoped records after logical deletion uses a
   `deleted_at` timestamp for the soft-delete marker unless the storage model
   requires a clearly documented equivalent.
2. `deleted_at = NULL` means the record is still active.
3. `deleted_at IS NOT NULL` means the record is logically deleted and eligible
   for retention-window processing.
4. Hard deletion is performed only by a separate purge job after the retention
   period expires.
5. The control plane must expose a GDPR erasure endpoint that marks tenant data
   for downstream purge workflows and emits an auditable event.

## Lifecycle

The expected lifecycle is:

1. Tenant is active.
2. Tenant is logically deleted by the control plane.
3. Retention metadata is updated to record the deletion timestamp.
4. Downstream workers consume the tombstone event and purge data after the
   configured retention window.

## Required Semantics

- `deleted_at` is the source of truth for logical deletion.
- API handlers must be idempotent where practical.
- Audit trails must record who requested the deletion and when it occurred.
- Physical deletion must never happen inline with the user-facing erasure call.

## Control-Plane API

The canonical public endpoint for GDPR workflows is:

```http
POST /api/control/tenants/{tenant_id}/gdpr-erasure
```

The endpoint must:

- Reject tenants that are not in `deleted` state.
- Reuse the audited tombstone pipeline.
- Write the downstream outbox event for purge consumers.
- Return the same timestamp on idempotent replays.

## Compatibility

Existing tombstone routes may remain available as aliases for internal callers.
The important requirement is that all entry points converge on the same audited
state transition and outbox emission path.
