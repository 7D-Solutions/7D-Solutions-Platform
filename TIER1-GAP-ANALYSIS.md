# Tier 1 Gap Analysis — Platform Constitution v1.3

**Date:** 2026-02-15
**Analyst:** Claude (automated codebase review)
**Constitution Version:** v1.3 (Locked)
**Scope:** Tier 1 items only. Tiers 2–5 explicitly deferred.

---

## Summary

| Category | Compliant | Partial | Missing | Violating |
|----------|-----------|---------|---------|-----------|
| One-Way Doors (8) | 1 | 5 | 2 | 0 |
| Two-Way Doors (6) | 1 | 2 | 3 | 0 |
| **Total (14)** | **2** | **7** | **5** | **0** |

---

## One-Way Doors (Critical — Cannot Be Changed Later)

### 1. Event Envelope Structure

**Status: PARTIAL**

The platform defines a shared `EventEnvelope<T>` in `platform/event-bus/src/envelope.rs`. All modules re-export and use this shared type. The following fields are present:

| Constitution Field | Codebase Status |
|---|---|
| `event_id` (UUID) | ✅ Present |
| `event_type` (string) | ❌ **Missing from envelope struct.** Event type is tracked in the outbox table column but is not a field on the EventEnvelope itself. |
| `schema_version` (integer) | ❌ **Missing entirely.** No schema versioning on events. |
| `occurred_at` (ISO 8601) | ✅ Present (DateTime\<Utc\>) |
| `producer` (string) | ✅ Present as `source_module` |
| `tenant_id` (UUID) | ✅ Present (stored as String, not UUID — minor type mismatch) |
| `trace_id` (UUID) | ❌ **Missing.** The envelope has `correlation_id` and `causation_id` (both Optional\<String\>), but no dedicated `trace_id` for distributed tracing. |
| `reverses_event_id` (UUID\|null) | ❌ **Missing from envelope.** |
| `supersedes_event_id` (UUID\|null) | ❌ **Missing from envelope.** |
| `side_effect_id` (UUID\|null) | ❌ **Missing from envelope.** |
| `replay_safe` (boolean) | ❌ **Missing from envelope.** |
| `mutation_class` (enum) | ❌ **Missing from envelope.** |
| `payload` (JSON) | ✅ Present (generic type parameter) |

**Gaps:** 7 of 13 required fields are missing from the EventEnvelope struct. The envelope carries only basic routing metadata (event\_id, occurred\_at, tenant\_id, source\_module, source\_version, correlation\_id, causation\_id, payload). The constitution requires the full envelope schema with all metadata fields present from first emission, even if not actively consumed. The `source_version` field exists but maps loosely to the constitution's versioning intent — it tracks the module's cargo package version, not the event schema version.

Additionally, `validate_envelope_fields()` has a bug: it validates `source_module` twice (once for `source_version`) rather than actually checking the `source_version` field.

---

### 2. Module Boundary Discipline

**Status: PARTIAL**

**What's compliant:**
- Modules are clearly separated into distinct directories under `modules/` (ar, gl, payments, subscriptions, notifications) and `platform/` (identity-auth, event-bus).
- No cross-module Cargo dependencies exist. Each module depends only on the shared `event-bus` crate. No module imports another module's code.
- Communication is via HTTP commands + NATS events. The subscriptions module communicates with AR via `AR_BASE_URL` HTTP environment variable. GL and Payments consume events via NATS subscriptions.
- Each module has its own dedicated PostgreSQL database (confirmed in docker-compose.infrastructure.yml: ar\_db, gl\_db, payments\_db, subscriptions\_db, notifications\_db). No cross-database reads.
- No shared ORM models between modules.

**Gaps:**
- **Domain ownership registry: Missing.** The constitution requires a domain ownership registry (markdown file at Tier 1). No such file exists in the codebase. There is no declaration of which module owns which domain concept.
- **Write degradation class declarations: Missing.** No inter-module command has a declared write degradation class. The subscriptions module calls AR via HTTP (`AR_BASE_URL`) but has no documented degradation behavior for when AR is unavailable.
- **Degradation integration tests: Missing.** No tests simulate target module unavailability to assert declared degradation behavior.

---

### 3. Per-Tenant DB Isolation

**Status: PARTIAL**

**What's implemented:**
- Each module gets a dedicated PostgreSQL instance (docker-compose.infrastructure.yml shows separate containers: 7d-ar-postgres, 7d-gl-postgres, 7d-payments-postgres, 7d-subscriptions-postgres, 7d-notifications-postgres). This is per-module isolation.
- Data is scoped by `app_id` (AR module) or `tenant_id` (GL, Payments) column in tables. This is row-level tenant scoping within a shared database.

**Gaps:**
- **Per-tenant database isolation is not implemented.** The constitution requires each tenant to get a dedicated database (Tier 2 isolation). The current architecture uses a single database per module with row-level tenant scoping via `app_id`/`tenant_id` columns. This is closer to Tier 1 shared-DB isolation, which is acceptable at Tier 1, but the one-way door is the connection resolver.
- **Centralized connection resolver function: Not fully implemented.** GL has a `db::init_pool()` function. However, AR's `main.rs` creates its connection pool directly with `PgPoolOptions::new()...connect()` inline. Payments, Subscriptions, and Notifications also create pools inline in `main.rs`. The constitution mandates all database connection creation must be routed through a single centralized resolver function per module. Most modules bypass this by creating pools directly. The resolver seam (the one-way door) is not consistently present.
- **Inconsistent tenant identifier naming.** AR uses `app_id`, GL and others use `tenant_id`. The constitution specifies `tenant_id` (UUID).

---

### 4. Mutation Class Declarations

**Status: MISSING**

No domain concept anywhere in the codebase is classified with a mutation class (strict\_immutable, compensating\_required, mutable\_with\_audit, or mutable\_standard). The enum does not appear in any code, configuration, documentation, or migration file. There is no evidence that the decision tree from Section 7 of the constitution has been applied.

Based on domain analysis, several concepts need classification:
- Invoices, Payments, Charges, Refunds → likely `strict_immutable` or `compensating_required`
- Journal Entries → `strict_immutable`
- Customers, Payment Methods → `mutable_with_audit`
- Subscriptions, Disputes → `compensating_required`

Without these declarations, the codebase cannot enforce the correct mutation behavior per class (e.g., preventing UPDATE/DELETE on strict\_immutable data).

---

### 5. Outbox Pattern

**Status: PARTIAL**

**What's compliant:**
- All five modules have an `events_outbox` table (confirmed via migration files).
- All modules have a background polling publisher that reads unpublished events from the outbox and publishes to the event bus (NATS or in-memory). AR polls every 1 second. GL and others use similar patterns.
- Consumer deduplication by `event_id` is implemented. AR has `processed_events` table with `is_event_processed()` check. GL uses `processed_repo::exists()`. Both check before processing and mark after.

**Gaps:**
- **AR module does NOT write outbox events in the same transaction as domain state changes.** In `routes.rs`, invoice finalization updates the invoice status via `sqlx::query(...).fetch_one(&db)` (not in a transaction), then separately calls `enqueue_event(&db, ...)` which does its own independent INSERT into `events_outbox`. These are two separate operations, not wrapped in a single transaction. If the process crashes after the state change but before the outbox write, the event is silently lost.
- **GL module IS compliant.** `journal_service::process_gl_posting_request()` wraps journal entry creation, balance updates, processed event marking, and (in reversal_service) outbox insertion all within a single `pool.begin()` ... `tx.commit()` transaction.
- **Subscriptions and Payments outbox atomicity not verified** — likely have the same gap as AR based on similar code patterns.
- **Publisher does not guarantee exactly-once delivery.** `mark_as_published` happens after `event_bus.publish()`. If the process crashes between publish and mark, the event will be re-published on restart (at-least-once is acceptable, but worth noting).

---

### 6. Reversal/Supersession Linkage

**Status: PARTIAL**

**What's compliant:**
- GL module implements reversal entries with `reverses_entry_id` in the journal entries table (migration `20260213000002_add_reverses_entry_id.sql`).
- `reversal_service.rs` creates inverse journal entries with swapped debit/credit and links them back to the original via `reverses_entry_id`.
- Max chain depth of 1 is enforced: the reversal service checks `if original_entry.reverses_entry_id.is_some()` and rejects reversal of reversals with `AlreadyReversed` error.

**Gaps:**
- **`reverses_event_id` and `supersedes_event_id` are missing from the EventEnvelope.** The GL module tracks reversal linkage at the journal entry (domain) level, not at the event level. The constitution requires these fields on the event envelope itself so that any consumer can detect reversal/supersession relationships from the event metadata alone.
- **`supersedes_event_id` is not implemented anywhere.** There is no supersession mechanism in any module. No event or domain record tracks data corrections that preserve historical accuracy.
- **Only GL has reversal linkage.** AR, Payments, Subscriptions, and Notifications have no reversal or supersession mechanisms.

---

### 7. Audit Event Emission

**Status: MISSING**

**What exists:**
- AR module has an `ar_events` table and `log_event()` / `log_event_async()` functions that write local event logs. These are fire-and-forget logging, not audit events in the constitutional sense.
- GL module has period close snapshots with deterministic hashing for audit trail integrity.

**What's missing:**
- **No audit events are emitted for strict\_immutable or compensating\_required mutations.** Since mutation classes aren't declared (see item 4), audit event emission by class is not implemented.
- **No change events with field-level diffs for mutable\_with\_audit concepts.** When AR updates a customer record, no change event is emitted with `{field, previous_value, new_value}` diffs.
- **Actor identification is incomplete.** AR's `log_event` has a `source` field but doesn't distinguish between user, system, and impersonation actions. The `update_source` and `updated_by` columns exist on some AR tables (customers, subscriptions) but are not consistently populated, and they don't produce audit events.

---

### 8. Event-Driven Projections

**Status: PARTIAL**

**What's compliant:**
- GL module builds materialized `account_balances` projections from journal entries. The `rebuild_balances` tool can deterministically recompute balances from journal entries (the source of truth).
- GL tracks `last_journal_entry_id` in the account\_balances table, which serves a similar purpose to `last_event_id` for freshness tracking.
- Idempotent consumption is implemented via `processed_events` tables in GL and AR.

**Gaps:**
- **No per-module projection tables sourced from events in most modules.** AR, Payments, Subscriptions, and Notifications read directly from their own domain tables. They don't maintain separate projection tables built from events.
- **`last_event_id` tracking is not implemented per the constitution's specification.** GL tracks `last_journal_entry_id` which is close but not event-driven. Other modules have no event position tracking at all.
- **HTTP fallback with timeout is not implemented.** No module implements the pattern of falling back to HTTP calls when projections are stale. There is no timeout budget, circuit breaker, or fallback rate threshold.
- **No projection freshness metrics.** The constitution requires `event_stream_position_gap`, `projection_lag_ms`, and `fallback_invocation_count` observability. None exist.

---

## Two-Way Doors (Important But Correctable)

### 9. Tenant Provisioning Sequence

**Status: MISSING**

No tenant provisioning script or runbook exists. The deployment runbook (`docs/architecture/DEPLOYMENT-RUNBOOK.md`) covers module deployment but not tenant creation. There is no scripted 7-step provisioning sequence (create tenant record, allocate database, apply schemas, register versions, seed data, activate, verify). Tenants appear to be created implicitly by using different `app_id` values in API calls.

---

### 10. Operational Endpoints

**Status: PARTIAL**

| Endpoint | AR | GL | Payments | Subscriptions | Notifications |
|---|---|---|---|---|---|
| `/health` | ✅ | ✅ | ✅ | ✅ | ✅ |
| `/ready` | ❌ | ❌ | ❌ | ❌ | ❌ |
| `/version` | ❌ | ❌ | ❌ | ❌ | ❌ |

All five modules expose `/api/health` returning basic status. Health endpoints return module name and version in the JSON body but do not check database connectivity (except AR which accepts the db pool as state but doesn't query it).

**Gaps:** `/ready` (DB connected, projections initialized) and `/version` (module name, version, schema version) endpoints are missing across all modules. The health endpoints don't verify database reachability.

---

### 11. Retention Class Declarations

**Status: MISSING**

No domain concept has a declared retention class (permanent, long, medium, short). The term "retention" appears only in documentation discussing backup retention windows, not data lifecycle classification. No configuration, code comments, or documentation files declare retention classes per the constitution.

---

### 12. Backup Baseline

**Status: PARTIAL**

- Docker volumes are used for database persistence (`7d-ar-pgdata`, `7d-gl-pgdata`, etc.).
- The nightly CI workflow (`nightly.yml`) runs E2E tests but does not include backup or restore operations.
- No automated daily backup script or cron job exists.
- No quarterly test restore procedure is documented or automated.

**Gap:** The constitution requires automated daily backups and quarterly test restores. Neither is implemented.

---

### 13. Lightweight Lint Rules

**Status: MISSING**

No lint scripts exist for:
- Cross-module import detection (grep for `../other-module` patterns)
- Raw DB connection creation outside resolver function
- Event schema validation against declared contracts

The Cargo.toml dependency analysis confirms no cross-module code imports exist today, but there is no automated check to prevent future violations.

---

### 14. Command Idempotency Keys

**Status: COMPLIANT**

AR module implements HTTP-level idempotency via middleware (`idempotency.rs`):
- Client-supplied `Idempotency-Key` header on write operations (POST, PUT, DELETE, PATCH).
- Keys stored in `ar_idempotency_keys` table with response body, status code, and 24-hour TTL (`expires_at`).
- On duplicate submission, the stored response is returned without re-execution.
- Scope limited to write operations; GET requests pass through.

This matches the constitution's Section 8.2 requirements. Other modules don't yet expose write HTTP APIs that other modules call, so the requirement currently applies only to AR.

---

## Priority Remediation Order

Based on one-way door severity and implementation cost:

1. **Event Envelope Structure** — Add the 7 missing fields to `EventEnvelope`. This is the single highest-priority item because every event emitted today is missing required metadata. Retrofitting becomes exponentially harder as event volume grows.

2. **Mutation Class Declarations** — Classify every domain concept before writing more code. Zero implementation cost, high documentation value.

3. **Outbox Atomicity (AR module)** — Wrap invoice finalization + outbox write in a single transaction. Silent event loss is a data integrity risk.

4. **Domain Ownership Registry** — Create the markdown registry. Zero implementation cost.

5. **Reversal/Supersession on Envelope** — Add `reverses_event_id` and `supersedes_event_id` to EventEnvelope. GL's domain-level linkage is good but insufficient.

6. **Connection Resolver Seam** — Extract inline `PgPoolOptions::new()` calls into per-module resolver functions. This is the one-way door for future isolation tier support.

7. **Audit Event Emission** — Implement after mutation classes are declared.

8. **Event-Driven Projections** — Implement `last_event_id` tracking and HTTP fallback. GL is closest to compliant.
