# Fireproof ERP → 7D Solutions Platform: Reuse Synthesis Report

**Author:** Claude Desktop Agent
**Date:** 2026-03-05
**Status:** Final
**Scope:** Cross-reference analysis of Fireproof ERP extraction candidates against the 7D Solutions Platform for manufacturing scope expansion.

---

## Executive Summary

After examining both codebases in depth, I find **5 concrete extraction candidates** and **2 pattern-only adaptations** worth pursuing. The 7D Platform already has substantial infrastructure (security, event-bus, RBAC) that narrows the gap compared to what a surface-level diff suggests. The highest-leverage extractions are the **organization hierarchy** and **inventory movement tracking** — these fill genuine platform gaps. Infrastructure items (error types, event helpers) offer smaller but low-risk wins. Security consolidation is complex and should be deferred until the platform security crate matures its scope-based authorization story.

---

## 1. Module-by-Module Extraction Assessment

### 1.1 Organization Hierarchy (Facility → Building → Zone)

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/organization/` (types.rs, service.rs, repository.rs) |
| **Platform counterpart** | `modules/inventory/src/domain/locations.rs`, `modules/inventory/db/migrations/20260218000012_create_locations.sql` |
| **Verdict** | **EXTRACT** |

**Gap analysis:** The platform's Inventory module has a flat `locations` table scoped to `warehouse_id` — no concept of physical site hierarchy above "warehouse." Fireproof built a three-tier model: `Facility → Building → Zone`, each with `code`, `name`, `is_active`, `display_order`, and timestamps. Storage locations sit under zones with a location_type taxonomy (`bin/shelf/rack/cabinet/drawer/room/other`). The repository includes JOIN chains that resolve a storage location's full hierarchy path (zone → building → facility).

**What to extract:**
- The `Facility`, `Building`, `Zone` models and their `WithCounts`/`WithParents` view structs
- The hierarchical SQL pattern (multi-table JOIN for resolved paths)
- Deactivation protection (`count_active_in_zone` prevents deactivating zones with live storage locations)

**Adaptation needed:**
- Replace Fireproof's `tenant_id: String` with platform's `tenant_id: Uuid` convention
- Replace `i32` primary keys with `Uuid` (platform convention)
- Wire into the existing `warehouses` table — a warehouse becomes a child of a facility, or sits alongside facilities as an alternative grouping
- Add outbox event emission for hierarchy changes (Fireproof has none — the platform pattern requires Guard → Mutation → Outbox)
- Scope-based RBAC: Fireproof uses `AuthzGate` with scope strings; platform uses `Role/Operation` enum pairs. The hierarchy CRUD should use the platform's `RequirePermissionsLayer`

**Effort:** ~3 days (migrations + models + service + HTTP handlers + tests)

---

### 1.2 Inventory Movement Tracking

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/inventory_movement/` (types.rs, service.rs, repository.rs) |
| **Platform counterpart** | None — platform Inventory module has no movement concept |
| **Verdict** | **EXTRACT** |

**Gap analysis:** The platform Inventory module tracks items at locations but has no movement history, no audit trail of where things were, no atomic move-with-evidence pattern. Fireproof built exactly this:

- `MovementRecord` — immutable evidence row (from/to location, quantity, reason, moved_by, timestamp)
- `CurrentLocation` — mutable projection (latest location per entity, updated atomically with each movement)
- Atomic `move_item()` — single transaction does: (1) look up current location, (2) insert movement record, (3) upsert current_location. If any step fails, all roll back.
- `history()` — flexible filtering by entity_type, entity_id, location_id with pagination
- `items_at_location()` — reverse lookup: what's at this storage location?

**What to extract:**
- The dual-table pattern (immutable movement log + mutable current-location projection)
- The atomic move transaction
- Entity-type polymorphism (`gauge`/`tool`/`part` with per-type quantity constraints)
- Movement history with flexible filtering

**Adaptation needed:**
- Replace `i32` location IDs with `Uuid` (platform convention)
- Replace `tenant_id: Uuid` in repository method signatures (Fireproof uses `&Uuid`, platform uses `&str` in some modules — standardize)
- Add outbox events for movements (production/quality modules will need to react to location changes)
- Integrate with the existing platform `locations` table rather than Fireproof's `storage_locations`
- Generalize entity_type beyond gauge/tool/part — manufacturing will need work-order, lot, fixture entity types

**Effort:** ~3 days (migrations + models + service + HTTP handlers + tests)

---

### 1.3 API Error Registry

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/error_registry.rs` |
| **Platform counterpart** | Per-module error enums (e.g., `WorkOrderError`, `LocationError`) with ad-hoc `IntoResponse` impls |
| **Verdict** | **EXTRACT** |

**Gap analysis:** Every platform module independently defines error types and implements `IntoResponse` for Axum. Fireproof extracted a shared `ApiError` type with:

- `ApiErrorBody` (code string + message + optional details map)
- Convenience constructors: `bad_request()`, `not_found()`, `conflict()`, `unprocessable()`, `internal()`
- Consistent HTTP status mapping
- `From<DomainValidationError>` bridge for the validation crate

The platform modules (production, inventory, quality-inspection) each reinvent this. A shared error type prevents drift as more modules are added for manufacturing.

**What to extract:**
- The `ApiError` / `ApiErrorBody` types
- Convenience constructors
- The pattern of bridging domain errors → API errors via `From` impls

**Adaptation needed:**
- Add this as a `platform/api-error` crate (or merge into an existing `platform/common` crate)
- Each module keeps its domain-specific error enum but gets a blanket `From<ModuleError> for ApiError` pattern
- Add correlation_id threading (the platform's outbox pattern already carries correlation IDs — errors should too)

**Effort:** ~1 day (crate + type definitions + From impls for existing modules)

---

### 1.4 Event Infrastructure Helpers

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/events/` (client.rs, idempotency.rs, dlq.rs) |
| **Platform counterpart** | `platform/event-bus/src/lib.rs` (EventBus trait, NatsBus, outbox, consumer_retry) |
| **Verdict** | **ADAPT-PATTERN** |

**Gap analysis:** The platform event-bus crate provides the core abstractions: `EventBus` trait, `NatsBus` implementation, `EventEnvelope` with validation, outbox pattern, and consumer retry. Fireproof's events module adds three app-level concerns on top:

1. **JetStream consumer management** (`client.rs`) — durable consumer creation per stream, health/status endpoints for ops
2. **Idempotency dedupe** (`idempotency.rs`) — `with_dedupe()` wraps a handler in a transaction with `(event_id, handler_name)` dedupe check, guaranteeing exactly-once over NATS at-least-once delivery
3. **DLQ failure classification** (`dlq.rs`) — `FailureClass` enum (Transient/Permanent) with `classify_error()` heuristic for retry decisions

The platform already has `consumer_retry` but lacks the idempotency dedupe and failure classification. These are genuinely useful patterns.

**What to adapt:**
- The idempotency dedupe pattern (`with_dedupe` using an `(event_id, handler_name)` table) should be offered as a helper in the platform event-bus crate
- The DLQ failure classification (Transient vs. Permanent) should inform the existing `consumer_retry` logic
- The JetStream consumer management is too app-specific to extract as-is, but the health check pattern is worth standardizing

**Why ADAPT-PATTERN, not EXTRACT:**
- The platform event-bus already exists with different abstractions — we can't drop in Fireproof's code without breaking the existing API
- The idempotency table schema and the failure classification logic are the valuable pieces, not the specific Rust code

**Effort:** ~2 days (add dedupe helper to event-bus crate + failure classification enum + update consumer_retry + migration for dedupe table)

---

### 1.5 Batch Workflow / Status Machine Pattern

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-gauge-domain/src/status_machine.rs` |
| **Platform counterpart** | `modules/production/src/domain/work_orders.rs` (WorkOrderStatus with inline match) |
| **Verdict** | **ADAPT-PATTERN** |

**Gap analysis:** Fireproof's gauge domain has a sophisticated 12-status state machine with:

- `ALLOWED_TRANSITIONS` as a const array (exhaustive transition matrix)
- `calculate_status()` with priority ordering across multiple inputs
- `can_checkout()` eligibility check
- Per-transition error codes for forbidden transitions

The platform's production module uses a simpler inline approach — `WorkOrderStatus` has 3 states (Draft/Released/Closed) with transitions validated via `if current != expected` checks in each method.

As manufacturing grows, more entities will need state machines: inspection plans, ECOs, routing operations, lot dispositions. A reusable state machine pattern prevents each module from reinventing transition validation.

**What to adapt:**
- The const-array transition matrix pattern (define allowed transitions declaratively, validate generically)
- The per-transition error code mapping
- The priority-based status calculation for entities with multiple status inputs

**Why ADAPT-PATTERN, not EXTRACT:**
- The Fireproof state machine is deeply coupled to gauge-specific statuses and calibration logic
- The valuable piece is the architectural pattern, not the specific code
- Each platform module will define its own status enum and allowed transitions, but can share the validation machinery

**Effort:** ~1 day (generic `StateMachine<S>` trait in a platform utility crate, refactor WorkOrderStatus to use it as proof-of-concept)

---

### 1.6 Security Middleware Consolidation

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/identity_auth/` and `crates/fireproof-erp/src/security/` |
| **Platform counterpart** | `platform/security/` (JWT, RBAC, rate limiting, claims middleware) |
| **Verdict** | **SKIP (for now)** |

**Gap analysis:** This is where the Fireproof agent and I disagree most. The platform security crate already provides:

- **JWT verification** (`JwtVerifier` with RS256, `VerifiedClaims` extraction)
- **RBAC** (`Role` enum: Admin/Operator/Auditor, `Operation` enum, `RbacPolicy`)
- **Rate limiting** (token bucket, tenant-aware, Prometheus metrics)
- **Claims middleware** (`ClaimsMiddleware`, `RequirePermissionsLayer`)
- **Service auth** (service-to-service token verification)
- **Webhook verification** (HMAC-based)

Fireproof adds on top:
- **CSRF protection** — the platform doesn't have this, but it's a frontend concern and the platform is backend-only
- **Audit logging middleware** — useful but belongs in an observability crate, not security
- **HIBP password checking** — vertical-specific, not platform material
- **Scope-based AuthzGate** — Fireproof uses scope strings (`gauges:read`, `calibrations:write`) vs. platform's role-based approach. This is a real architectural gap but not one that should be resolved by extracting Fireproof's implementation.
- **RequestContext extraction** — platform already has `VerifiedClaims` which serves the same purpose

**Why SKIP:**
- The platform security crate is actively maintained and has different design decisions (role-based vs. scope-based auth)
- Extracting Fireproof's security layer would create a competing abstraction
- The genuine gaps (audit logging, scope-based permissions) should be addressed as platform security crate enhancements, not Fireproof extractions
- CSRF is irrelevant for a backend-only platform

**Recommendation:** File enhancement requests against the platform security crate for: (1) audit log middleware, (2) scope-based permission model alongside the existing role model. These are better built fresh with platform conventions than extracted from Fireproof.

---

### 1.7 Fireproof Validation Crate

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-validation/src/` (error.rs, thread.rs) |
| **Platform counterpart** | Per-module inline validation (e.g., `if req.tenant_id.trim().is_empty()` in every repo method) |
| **Verdict** | **SKIP** |

**Why SKIP:**
- The Fireproof validation crate is dominated by thread-gauge-specific logic (`validate_thread_fields`, `normalize_thread_data`, `VALID_THREAD_TYPES`)
- The generic pieces (`DomainValidationError`, `ValidationErrorCode`, `CorrectUsage`) are useful but tiny — ~50 lines of type definitions
- The platform already has validation patterns baked into each module; extracting a shared validation error type is better done as part of the API Error Registry extraction (item 1.3) rather than as a separate crate
- The `From<DomainValidationError> for ApiError` bridge in the error registry already handles the cross-cutting concern

---

### 1.8 Maintenance Facade Pattern

| Field | Value |
|-------|-------|
| **Fireproof source** | `crates/fireproof-erp/src/maintenance/` (facade.rs, client.rs) |
| **Platform counterpart** | N/A (this is the vertical → platform integration pattern) |
| **Verdict** | **SKIP (correct as-is)** |

**Why SKIP:**
- The maintenance facade is a thin HTTP client that maps Fireproof concepts (gauges, calibrations) to 7D Platform maintenance concepts (assets, work orders). This is exactly what a Tier 3 vertical should look like.
- The retry logic (exponential backoff) in `client.rs` is useful but trivial — any HTTP client library provides this.
- Extracting this would be extracting Fireproof-specific business logic, which violates the tier model.

---

### 1.9 Frontend UI Kit

| Field | Value |
|-------|-------|
| **Fireproof source** | `frontend/` (React components, Tailwind) |
| **Platform counterpart** | None (platform is backend-only) |
| **Verdict** | **SKIP** |

**Why SKIP:**
- The platform CLAUDE.md explicitly states: "This is a backend-only platform repo. Verticals build their own frontends in separate repos."
- Fireproof's frontend components (quoting BOM, gauge management) are vertical-specific
- A shared UI kit is a valid future goal but premature before the platform API contracts stabilize through the manufacturing buildout

---

## 2. Priority Ordering

| Priority | Item | Verdict | Effort | Rationale |
|----------|------|---------|--------|-----------|
| **P0** | Organization Hierarchy | EXTRACT | 3 days | Blocks location-aware manufacturing workflows; no platform equivalent exists |
| **P0** | Inventory Movement Tracking | EXTRACT | 3 days | Blocks WIP tracking, lot traceability, quality hold flows; no platform equivalent |
| **P1** | API Error Registry | EXTRACT | 1 day | Low effort, high consistency payoff; prevents drift as manufacturing modules multiply |
| **P1** | Event Idempotency + DLQ Patterns | ADAPT-PATTERN | 2 days | De-risks event-driven manufacturing flows; complements existing platform event-bus |
| **P2** | Status Machine Pattern | ADAPT-PATTERN | 1 day | Useful as manufacturing entities multiply; not blocking for Phase 0–1 |

**Total estimated effort:** ~10 engineering days

**What's NOT on this list** (and why):
- Security consolidation — platform crate is ahead; file enhancement requests instead
- Validation crate — too domain-specific; generic pieces fold into API Error Registry
- Maintenance facade — correct as vertical code; not extractable
- Frontend UI kit — premature; platform is backend-only

---

## 3. Dependency Map

```
                    ┌─────────────────────────┐
                    │   platform/api-error     │  (P1 — new crate)
                    │   ApiError, ApiErrorBody │
                    └────────┬────────────────┘
                             │ used by
                    ┌────────▼────────────────┐
                    │   platform/event-bus     │  (P1 — enhanced)
                    │   + dedupe helper        │
                    │   + failure class enum   │
                    └────────┬────────────────┘
                             │ used by
          ┌──────────────────┼──────────────────┐
          │                  │                  │
  ┌───────▼──────┐  ┌───────▼──────┐  ┌───────▼──────┐
  │  modules/    │  │  modules/    │  │  modules/    │
  │  inventory   │  │  production  │  │  quality-    │
  │  + org hier  │  │  (existing)  │  │  inspection  │
  │  + movements │  │              │  │  (existing)  │
  └──────────────┘  └──────────────┘  └──────────────┘
```

**Dependency constraints:**

1. **API Error Registry** has no upstream dependencies — can ship first.
2. **Event helpers** depend on the existing event-bus crate only — can ship in parallel with API Error Registry.
3. **Organization Hierarchy** depends on API Error Registry (for consistent error responses) and the existing inventory module's migrations (warehouse table FK relationships). Must ship before or alongside inventory movement.
4. **Inventory Movement** depends on Organization Hierarchy (movement targets are locations within the hierarchy) and Event helpers (movements should emit events for downstream consumers).
5. **Status Machine Pattern** is a standalone utility with no hard dependencies — can ship at any point.

**Suggested sequencing:**

```
Week 1:  API Error Registry + Event helpers (parallel, no dependencies)
Week 2:  Organization Hierarchy (depends on API Error Registry)
Week 3:  Inventory Movement (depends on Org Hierarchy + Event helpers)
Week 4+: Status Machine Pattern (whenever convenient)
```

---

## 4. Risk Assessment

### 4.1 High Risk: Org Hierarchy vs. Warehouse Model Collision

**Risk:** The platform Inventory module already has a `warehouses` table and `locations` table scoped to warehouses. Introducing Facility → Building → Zone creates a potential conflict — do warehouses sit inside facilities? Are they parallel concepts? Do existing warehouse-scoped queries break?

**Mitigation:**
- Define the relationship explicitly before coding: a `warehouse` is a logical inventory grouping (can span physical locations); a `facility` is a physical site. They are orthogonal.
- Add an optional `facility_id` FK to the `warehouses` table (nullable, non-breaking migration)
- Existing warehouse/location queries continue to work unchanged
- New manufacturing workflows can optionally resolve the physical hierarchy

### 4.2 Medium Risk: Movement Tracking Entity-Type Explosion

**Risk:** Fireproof tracks movements for 3 entity types (gauge/tool/part) with hardcoded quantity constraints. Manufacturing needs many more: work-order, lot, fixture, die, sample, container. Hardcoding per-type rules won't scale.

**Mitigation:**
- Extract entity type definitions into a configuration table or enum registry, not hardcoded match arms
- Quantity constraints should be per-entity-type configuration, not code
- The movement table's `entity_type` column should be a free string with a CHECK constraint against the registry, not a Rust enum

### 4.3 Medium Risk: Event Idempotency Table Growth

**Risk:** The dedupe table (`(event_id, handler_name)`) grows monotonically. At manufacturing scale with many event types and handlers, this table could become large.

**Mitigation:**
- Add a TTL-based cleanup job (events older than 7 days are safe to remove — NATS JetStream's max delivery window is configurable but typically shorter)
- Partition by date or use a hash-based approach with a sliding window
- Monitor table size in the Prometheus metrics already present in the platform

### 4.4 Low Risk: API Error Crate Adoption Friction

**Risk:** Existing modules have their own error types with module-specific variants. Introducing a shared `ApiError` doesn't remove the need for domain errors — it adds a layer.

**Mitigation:**
- Don't replace module error enums — add `From<ModuleError> for ApiError` impls
- The shared type only governs the HTTP response shape, not internal error handling
- Migration can be gradual: new modules use it from day one, existing modules adopt it as they're touched

### 4.5 Low Risk: Status Machine Over-Engineering

**Risk:** A generic `StateMachine<S>` trait may be overkill if most modules only have 3–5 states with simple linear progressions.

**Mitigation:**
- Start with a minimal trait: `fn allowed_transitions(&self) -> &[(S, S)]` and `fn validate_transition(from: S, to: S) -> Result<(), TransitionError>`
- Don't build priority-based status calculation or compound state logic until a module actually needs it
- The WorkOrder refactor (Draft → Released → Closed) is the proof-of-concept — if the trait adds complexity without clarity, abandon it

---

## 5. Effort Estimates

| Item | New Code | Migration | Tests | Integration | Total |
|------|----------|-----------|-------|-------------|-------|
| **Org Hierarchy** | 1.5 days | 0.5 days | 0.5 days | 0.5 days | **3 days** |
| **Inventory Movement** | 1.5 days | 0.5 days | 0.5 days | 0.5 days | **3 days** |
| **API Error Registry** | 0.5 days | — | 0.25 days | 0.25 days | **1 day** |
| **Event Helpers** | 1 day | 0.25 days | 0.5 days | 0.25 days | **2 days** |
| **Status Machine** | 0.5 days | — | 0.25 days | 0.25 days | **1 day** |
| **Total** | | | | | **10 days** |

**Assumptions:**
- One experienced Rust developer
- Platform CI/CD pipeline already works (no infra setup)
- Integration testing against real Postgres (per Fireproof's convention; the platform may differ)
- Does not include review cycles or cross-team coordination overhead

**What's NOT included:**
- Refactoring existing modules to adopt the new patterns (gradual, ongoing)
- Documentation beyond inline doc comments
- Performance benchmarking of the movement tracking at scale

---

## Appendix A: File Cross-Reference

| Fireproof Source | Platform Target | Action |
|------------------|-----------------|--------|
| `src/organization/types.rs` | `modules/inventory/src/domain/org_hierarchy.rs` (new) | EXTRACT |
| `src/organization/service.rs` | `modules/inventory/src/domain/org_hierarchy.rs` (new) | EXTRACT |
| `src/organization/repository.rs` | `modules/inventory/src/domain/org_hierarchy.rs` (new) | EXTRACT |
| `src/inventory_movement/types.rs` | `modules/inventory/src/domain/movements.rs` (new) | EXTRACT |
| `src/inventory_movement/service.rs` | `modules/inventory/src/domain/movements.rs` (new) | EXTRACT |
| `src/inventory_movement/repository.rs` | `modules/inventory/src/domain/movements.rs` (new) | EXTRACT |
| `src/error_registry.rs` | `platform/api-error/src/lib.rs` (new crate) | EXTRACT |
| `src/events/idempotency.rs` | `platform/event-bus/src/dedupe.rs` (new module) | ADAPT |
| `src/events/dlq.rs` | `platform/event-bus/src/failure.rs` (new module) | ADAPT |
| `crates/fireproof-gauge-domain/src/status_machine.rs` | `platform/state-machine/src/lib.rs` (new crate) | ADAPT |
| `src/identity_auth/` | — | SKIP |
| `src/security/` | — | SKIP |
| `crates/fireproof-validation/` | — | SKIP |
| `src/maintenance/` | — | SKIP |
| `frontend/` | — | SKIP |

## Appendix B: Comparison with Fireproof Agent Analysis

The Fireproof-side agent identified 8 extraction candidates across 3 tiers. Here is how this report's findings align:

| Fireproof Agent Item | This Report | Alignment |
|----------------------|-------------|-----------|
| 1. API Error Types → shared crate | 1.3 API Error Registry | **Agree** |
| 2. Auth/RBAC → platform security | 1.6 Security (SKIP) | **Disagree** — platform already has this; file enhancements instead |
| 3. Security middleware → platform | 1.6 Security (SKIP) | **Disagree** — CSRF irrelevant for backend-only; audit log should be platform enhancement |
| 4. Validation → split shared/gauge | 1.7 Validation (SKIP) | **Partially disagree** — generic pieces fold into API Error Registry |
| 5. Event helpers → platform bus | 1.4 Event Helpers | **Agree** |
| 6. Org hierarchy → platform | 1.1 Organization Hierarchy | **Agree** |
| 7. Inventory movement → platform | 1.2 Inventory Movement | **Agree** |
| 8. Frontend UI kit → shared | 1.9 Frontend (SKIP) | **Agree on skip** — premature |

**Key difference:** This report adds the **Status Machine Pattern** (item 1.5) which the Fireproof agent did not identify, and **deprioritizes security consolidation** based on discovery that the platform security crate is more capable than the Fireproof agent's analysis implied.

---

*End of report.*
