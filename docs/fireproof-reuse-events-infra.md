# Fireproof ERP Reuse: Events Infrastructure, DLQ, and Idempotency

**Investigator:** DarkOwl
**Bead:** bd-3fjnp
**Date:** 2026-03-05

---

## Executive Summary

Fireproof ERP built a consumer-side event infrastructure that the 7D Platform currently lacks. The platform has strong **producer-side** infrastructure (EventEnvelope, outbox pattern, NATS publish, validation). But there is no standardized way for a service to **consume** events — no handler registry, no idempotency guard, no DLQ, no event router. Every module that needs to react to cross-module events would have to build these from scratch.

Fireproof's events/ directory (1,191 LOC) solves this gap cleanly. The recommendation is to **extract** the consumer-side infrastructure into a new platform crate, and **adapt the pattern** of the DLQ ops routes into a shared operations toolkit.

---

## Component-by-Component Assessment

### 1. EventClient (client.rs — 407 LOC)

**What it does:** JetStream durable consumer manager. Connects to NATS, creates/attaches pull consumers per stream, runs a consume loop that deserializes `EventEnvelope<Value>`, dispatches through the router, and handles ack/shutdown.

**Already in platform?** No. The platform `event-bus` crate provides:
- `EventBus` trait (publish + subscribe) — a raw pub/sub abstraction
- `NatsBus` — wraps `async_nats::Client` with simple publish/subscribe
- `connect_nats()` — URL parsing with auth extraction

The platform has **no** JetStream consumer management, no durable consumer lifecycle, no structured message dispatch loop.

**Is it generic?** Yes. The only Fireproof-specific part is the `SUBSCRIBED_STREAMS` config (3 stream names). The client itself is completely generic — it takes a `NatsConfig` and a `HandlerRegistry`.

**Recommendation: EXTRACT**

Extract as `platform/event-consumer/` or extend `platform/event-bus/` with a consumer module. Changes needed:
- Replace `SUBSCRIBED_STREAMS` with a config-driven stream list (each service declares what it subscribes to)
- Remove the Fireproof `NatsConfig` — use existing `connect_nats()` from platform
- Keep the health/status endpoints as-is (generic ops tooling)

**LOC estimate:** ~350 LOC liftable, ~50 LOC adaptation.

---

### 2. HandlerRegistry (registry.rs — 198 LOC)

**What it does:** Builder-pattern registry mapping `(event_type, schema_version)` to async handler functions. Immutable after construction. Lookup returns Found/UnknownVersion/UnknownType — distinguishing "skip" from "DLQ" cases.

**Already in platform?** No. Each module has its own outbox publisher, but there is no consumer-side handler dispatch pattern.

**Is it generic?** Completely. No Fireproof-specific types. The `HandlerFn` signature takes `(HandlerContext, JsonValue)` — any service can register handlers.

**Recommendation: EXTRACT**

Lift as-is into the platform consumer crate. Zero changes needed. The builder pattern with panic-on-duplicate-registration is the right safety check for wiring bugs.

**LOC estimate:** ~198 LOC, no adaptation needed.

---

### 3. Event Router (router.rs — 201 LOC)

**What it does:** Validates incoming `EventEnvelope<Value>` (checks non-empty tenant_id), builds `HandlerContext` with tracing metadata, dispatches through the registry. Returns `RouteOutcome` enum: Handled/Skipped/DeadLettered/Invalid/HandlerError.

**Already in platform?** No.

**Is it generic?** Yes. Uses the platform's own `EventEnvelope` type. The `HandlerContext` extraction is exactly what every consumer needs.

**Recommendation: EXTRACT**

Lift as-is. The `RouteOutcome` enum is well-designed — it distinguishes between "unknown event type" (skip, don't DLQ) and "known type but unknown version" (DLQ). This prevents poison-pill scenarios where a new event type crashes old consumers.

**LOC estimate:** ~201 LOC, no adaptation needed.

---

### 4. HandlerContext (context.rs — 26 LOC)

**What it does:** Struct carrying tenant_id, actor_id, correlation_id, causation_id, event_id, schema_version, received_at — extracted from the envelope so handlers don't parse raw metadata.

**Already in platform?** No.

**Is it generic?** Yes.

**Recommendation: EXTRACT**

Lift as-is. Consider adding `source_module` to the context so handlers know which module produced the event.

**LOC estimate:** ~26 LOC.

---

### 5. Idempotency Guard (idempotency.rs — 94 LOC)

**What it does:** `with_dedupe()` function that wraps handler execution in a Postgres transaction. Atomically checks an `event_dedupe` table for `(event_id, handler_name)`, runs the handler if not seen, inserts the dedupe row, and commits. Guarantees exactly-once side effects over at-least-once NATS delivery.

**Already in platform?** No. Individual modules have their own outbox tables with `published_at` tracking (producer-side), but there is no consumer-side idempotency pattern. The platform has no `event_dedupe` table or equivalent.

**Is it generic?** Yes. The `with_dedupe()` function takes a `PgPool`, event_id, handler_name, tenant_id, and a closure that receives a transaction. Any module's consumer handler can use this.

**Is this something every module should use?** Yes, absolutely. Any cross-module event handler that performs side effects (DB writes, state transitions) needs idempotency. Without it, NATS redelivery causes duplicate processing. The alternative — each module building its own dedupe — leads to inconsistent implementations and bugs.

**Recommendation: EXTRACT**

This is the highest-value component. Lift into the platform consumer crate. Each service that consumes events needs an `event_dedupe` table — provide a migration template. The handler closure signature (receives `Transaction`, returns `Transaction`) is elegant and forces atomic commit of handler effects + dedupe record.

**LOC estimate:** ~94 LOC, no adaptation needed. Migration script needed (~15 LOC SQL).

---

### 6. DLQ Handler (dlq.rs — 252 LOC)

**What it does:** Records handler failures to an `event_dlq` table with full replay context (event_id, handler_name, tenant_id, event_type, schema_version, payload, error_text). Classifies failures as transient (timeout, connection) or permanent (validation, bug). Provides list/get/mark_replayed queries. SHA-256 payload hash for audit trail.

**Already in platform?** Partially. The platform has a `consumer_retry.rs` (110 LOC) in event-bus with exponential backoff retry logic. But this is retry-before-DLQ — it has no persistent DLQ storage, no failure classification, no replay API.

**Relationship to platform retry:** Complementary. Platform retry handles transient failures inline (retry 3x with backoff). Fireproof DLQ handles failures that survive retries — persistent storage for investigation and manual replay. The two should work together: retry first, DLQ on exhaustion.

**Is it generic?** Yes. No Fireproof-specific types.

**Recommendation: EXTRACT**

Lift into the platform consumer crate. Each consuming service needs an `event_dlq` table — provide a migration template. Wire it into the consumer loop: after `consumer_retry` exhausts attempts, call `dlq::record_failure()`.

**LOC estimate:** ~252 LOC, minimal adaptation (wire to platform retry). Migration script needed (~25 LOC SQL).

---

### 7. DLQ Ops Routes (routes/ops_dlq.rs — 183 LOC)

**What it does:** Axum REST endpoints for DLQ inspection and replay:
- `GET /ops/dlq` — list failed events, filterable by tenant/handler
- `POST /ops/dlq/{id}/replay` — re-run handler with original payload, mark replayed

**Already in platform?** No. The platform's existing DLQ replay drill (bd-3colh) is a script-based approach. These HTTP endpoints would provide runtime operability.

**Is it generic?** Mostly. Uses the HandlerRegistry for replay dispatch. Would need to be wired per-service (each service has its own registry and DLQ table).

**Recommendation: ADAPT-PATTERN**

Don't lift as a shared crate — each service mounts its own ops routes. But provide the pattern as a template or utility functions that services can compose. The replay logic (lookup handler in registry, rebuild context, re-invoke, mark replayed) is generic and should be standardized.

**LOC estimate:** ~183 LOC pattern reference; each service writes ~50 LOC to mount.

---

### 8. Config/Events (config/events.rs — 82 LOC)

**What it does:** Defines `SUBSCRIBED_STREAMS` (stream name + filter subject pairs), `NatsConfig` with `from_env()`, and credential embedding into NATS URLs.

**Already in platform?** Partially. `connect_nats()` in `platform/event-bus/src/connect.rs` already handles URL credential extraction. The `NatsConfig::from_env()` and `embed_credentials()` are Fireproof-specific but solve the same problem.

**Recommendation: SKIP (credential embedding)** — platform already has `connect_nats()`. **ADAPT-PATTERN (stream config)** — each service needs to declare its subscribed streams; standardize the config shape.

**LOC estimate:** ~20 LOC pattern for stream config declaration.

---

### 9. Projections (projections/auth_activity.rs — 126 LOC)

**What it does:** Concrete example of a handler that uses `with_dedupe()` to idempotently project auth events into a `proj_auth_activity` table. Shows the pattern: parse payload → call `with_dedupe()` → insert projection row inside transaction → commit.

**Already in platform?** No projection framework exists. Modules have outbox publishers but no consumer-side projection pattern.

**Is it generic?** The code itself is Fireproof-specific (auth events → projection table). But the **pattern** is exactly what every cross-module consumer will follow.

**Recommendation: ADAPT-PATTERN**

Don't extract the auth_activity code. Do document the pattern as the canonical way to write idempotent event handlers. Include it in the platform consumer crate's documentation/examples.

**LOC estimate:** 0 LOC to extract; pattern documentation only.

---

## Gap Analysis: What the Platform is Missing

| Capability | Producer Side (Platform) | Consumer Side (Platform) | Fireproof Fills Gap? |
|---|---|---|---|
| Event envelope | EventEnvelope struct | N/A | N/A |
| Envelope validation | validate_and_serialize_envelope | N/A | N/A |
| NATS publish | NatsBus::publish | N/A | N/A |
| Outbox enqueue | Per-module outbox tables | N/A | N/A |
| Outbox polling/publish | Per-module background tasks | N/A | N/A |
| NATS connection | connect_nats() with auth | Same | No (already exists) |
| JetStream consumers | None | None | YES — EventClient |
| Handler dispatch | None | None | YES — Registry + Router |
| Idempotency guard | None | None | YES — with_dedupe() |
| Failure classification | None | None | YES — DLQ classify_error() |
| DLQ persistent storage | None | None | YES — event_dlq table |
| DLQ replay API | Script-based (bd-3colh) | None | YES — ops_dlq routes |
| Retry before DLQ | None | consumer_retry.rs | Complementary |

**The platform has a complete event production pipeline but zero standardized event consumption infrastructure.** Every new module that needs to react to events from other modules would need to build its own consumer loop, handler dispatch, idempotency, and DLQ handling. Fireproof already solved this.

---

## Mapping to Manufacturing Roadmap

| Roadmap Phase | Needs Consumer Infrastructure? | Why |
|---|---|---|
| Phase B (Production) | YES | Production module consumes BOM events, inventory issue confirmations. Work order completion triggers GL posting via events. |
| Phase C1 (Receiving Inspection) | YES | Receiving inspection is triggered by `item.received` events from Inventory. Auto-create inspection record on receipt. (bd-986e4 explicitly describes this bridge.) |
| Phase C2 (In-process/Final Inspection) | YES | In-process inspection triggered by operation completion events from Production. |
| Phase D (ECO) | YES | ECO approval triggers BOM revision activation, inventory disposition changes — all via events. |
| Phase E (Maintenance Consumption) | YES | Maintenance workcenter events trigger Production module for capacity/scheduling awareness. |

**Every manufacturing phase from B onward needs consumer-side infrastructure.** Without it, each phase would independently reinvent handler dispatch and idempotency.

---

## Recommended Extraction Plan

### New crate: `platform/event-consumer/`

**Contents:**
1. `client.rs` — JetStream consumer manager (from EventClient)
2. `registry.rs` — HandlerRegistry + RegistryBuilder (as-is)
3. `router.rs` — Event router + RouteOutcome (as-is)
4. `context.rs` — HandlerContext (as-is, add source_module)
5. `idempotency.rs` — with_dedupe() (as-is)
6. `dlq.rs` — DLQ recording + queries (as-is)
7. `migrations/` — SQL templates for event_dedupe and event_dlq tables

**Dependencies:** `event-bus` (for EventEnvelope, connect_nats), `async-nats`, `sqlx`, `serde_json`, `tokio`

**Total extractable LOC:** ~1,121 LOC (of 1,191 original)
**Adaptation LOC:** ~100 LOC (config generalization, platform integration)

### Recommended Beads

1. **Extract event-consumer crate** (P1, ~2-3 hours) — Create `platform/event-consumer/` with registry, router, context, idempotency, DLQ. Include migration templates. Wire to existing `consumer_retry` for retry-then-DLQ flow.

2. **DLQ ops route template** (P2, ~1 hour) — Document the DLQ ops route pattern. Provide utility functions for replay dispatch that services compose into their own route trees.

3. **Retrofit existing DLQ drill** (P2, ~1 hour) — Update the existing DLQ replay drill automation (bd-3colh area) to use the new structured DLQ tables instead of ad-hoc scripts.

---

## Could This Replace Per-Module Outbox Polling?

No, and it shouldn't. The outbox pattern (producer-side) and the consumer infrastructure (consumer-side) solve different problems:

- **Outbox** ensures events are reliably published even if NATS is temporarily down. It's a producer concern — atomicity between domain mutation and event publication.
- **Consumer infrastructure** ensures events are reliably processed on the receiving end — idempotency, routing, DLQ.

These are complementary, not competing. Both are needed for reliable event-driven architecture.

---

## Key Risks

1. **Schema migration coordination:** Each service that adopts event-consumer needs `event_dedupe` and `event_dlq` tables. Must be included in service-specific migrations, not a shared DB.

2. **Handler signature coupling:** The `HandlerFn` type uses `Arc<dyn Fn(HandlerContext, JsonValue) -> Pin<Box<dyn Future<...>>>>`. This is flexible but loses type safety on payloads. Handlers must parse JSON internally. Consider providing a typed wrapper macro in the future but don't let that block extraction.

3. **DLQ table per service vs shared:** Each service should have its own DLQ table (same DB as its data). Don't create a centralized DLQ service — that would introduce a cross-service dependency.
