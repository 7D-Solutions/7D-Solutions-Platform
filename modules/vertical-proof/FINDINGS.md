# Vertical Wiring Test — Findings

> Bead: bd-lz937  
> Agent: CopperRiver  
> Date: 2026-04-02  
> Objective: Build a vertical that calls 5 platform modules via SDK and document every gap.

## Summary

The plug-and-play SDK wiring **works**. A vertical can declare `[platform.services]`
in `module.toml`, depend on the typed client crates, and call `ctx.platform_client::<T>()`
to get fully wired HTTP clients with automatic URL resolution, header injection, and retry.

The compilation experience was clean — once the gaps below were addressed. Below are all
friction points encountered, ordered by severity.

---

## Gap 1: `clients/notifications` missing from workspace (BLOCKER)

**Problem:** The `platform-client-notifications` crate exists in `clients/notifications/`
with a valid Cargo.toml and PlatformService impls, but was not listed in `Cargo.toml`
workspace members. Any vertical depending on it would get:

```
error: package `platform-client-notifications` is not a member of the workspace
```

**Fix applied:** Added `"clients/notifications"` to workspace members.

**Also missing:** `clients/consolidation` — same issue.

**Root cause:** Client crates were generated but the workspace member list was not
updated to include all of them.

**Recommendation:** Add a CI check that every `clients/*/Cargo.toml` is in the workspace.

---

## Gap 2: No consumer guide documents which event subjects exist

**Problem:** To subscribe to an AR event, a vertical developer must guess the NATS
subject (e.g. `ar.events.invoice.opened`). There is no canonical list of event subjects
published by each module.

**Impact:** Vertical developers must read module source code to discover events.

**Recommendation:** Each module's `module.toml` should list its published event types
(already partially done via `[events.publish]`), and the consumer guide should enumerate
all subjects across the platform.

---

## Gap 3: Request types require full construction — no Default impls

**Problem:** Types like `CreateCompanyRequest` (21 fields), `CreateInvoiceRequest`
(13 fields), and `CreateItemRequest` (10 fields) require explicit construction of
every field. Since client types are auto-generated without `#[derive(Default)]`, the
vertical developer must write verbose struct literals with many `None` values.

**Impact:** Verbose boilerplate for every API call. Error-prone for large structs.

**Recommendation:** Add `#[derive(Default)]` to all `Create*Request` types in the
codegen. For types with required fields, consider a builder pattern or at minimum
document which fields are truly required vs optional.

---

## Gap 4: `PlatformClient::service_claims()` requires knowing the tenant ID pattern

**Problem:** For background tasks and event consumers that don't originate from an HTTP
request, verticals need to construct `VerifiedClaims`. The SDK provides
`PlatformClient::service_claims(tenant_id)`, but:

- It's on `PlatformClient`, not on `ModuleContext` — not discoverable
- No guidance on what `user_id` to use for service calls (currently `Uuid::nil()`)
- No helper to extract tenant_id from an EventEnvelope for consumer handlers

**Recommendation:** Add `ctx.service_claims(tenant_id)` as a convenience method on
`ModuleContext`. Document the service-call pattern in the consumer guide.

---

## Gap 5: No event schema types shared between publisher and consumer

**Problem:** When subscribing to `ar.events.invoice.opened`, the vertical must know
the payload schema. There are no shared event payload types in the client crates — the
consumer gets `EventEnvelope<serde_json::Value>` and must deserialize manually.

**Impact:** Fragile string-based deserialization. Schema changes break silently.

**Recommendation:** Each client crate should export event payload types (e.g.
`platform_client_ar::events::InvoiceOpened`) so consumers get typed payloads.
The SDK's `tenant_consumer` builder method partially addresses this but requires
the type to exist.

---

## Gap 6: Docker service names differ from manifest default_url patterns

**Problem:** The manifest `default_url` values use `http://localhost:PORT` for local
dev, but in Docker the services are at `http://7d-{module}:PORT`. A vertical developer
must set env vars (e.g. `PARTY_BASE_URL`) in Docker or update default_url values.

**Impact:** Works in local dev, fails in Docker without env var overrides.

**Recommendation:** Document this clearly in the consumer guide. Consider a naming
convention or Docker Compose env template that auto-sets `*_BASE_URL` vars.

---

## Gap 7: Outbox table schema must be manually created

**Problem:** The SDK auto-publishes from the outbox table declared in `module.toml`,
but the vertical must create the table itself via a migration. There's no SDK helper
or standard migration for the outbox table.

**Impact:** Every new module writes the same `events_outbox` DDL. Easy to get wrong.

**Recommendation:** Provide a standard migration in the SDK or a `sqlx::migrate!`
snippet that modules can include. Alternatively, have the SDK auto-create the outbox
table if it doesn't exist (with the expected schema).

---

## What Worked Well

1. **`ctx.platform_client::<T>()` works exactly as advertised.** Declare the service
   in `module.toml`, depend on the client crate, and the SDK handles URL resolution,
   header injection, and retry. Zero hand-written HTTP.

2. **Type safety is excellent.** The generated typed clients catch API misuse at compile
   time — wrong field types, missing required fields, incorrect path parameters.

3. **The `ModuleBuilder` pattern is clean.** `from_manifest → routes → consumer → run`
   is intuitive and requires minimal boilerplate for a working module.

4. **Service token injection is automatic.** When `SERVICE_AUTH_SECRET` is set, all
   platform client calls are authenticated. In dev mode, calls proceed unauthenticated
   with a clear log warning.

5. **The consumer wiring via `.consumer(subject, handler)` is simple and correct.**
   No manual NATS subscription management needed.

---

## Files Created

| File | Purpose |
|------|---------|
| `modules/vertical-proof/Cargo.toml` | Crate manifest with all 5 client deps |
| `modules/vertical-proof/module.toml` | SDK manifest with 5 platform services |
| `modules/vertical-proof/src/main.rs` | ModuleBuilder startup with consumer + routes |
| `modules/vertical-proof/src/lib.rs` | Library root with test_claims helper |
| `modules/vertical-proof/src/wiring_test.rs` | Wiring tests for all 5 modules + outbox |
| `modules/vertical-proof/db/migrations/20260402000001_init.sql` | Outbox table DDL |
| `modules/vertical-proof/FINDINGS.md` | This file |
