# Plan: Platform Contract Verification + Operational Vitals API

**Bead:** bd-3j1ri  
**Status:** Draft — ready for adversarial review  
**Author:** LavenderWaterfall  
**Date:** 2026-04-22

---

## 1. Problem Statement

Verticals and operators have no unified surface to answer these questions after tenant provisioning:

| Question | Current state |
|---|---|
| Did all modules finish provisioning for this tenant? | Partial — control-plane has `/api/control/tenants/{id}/provisioning` but only shows step-level, not event-consumer confirmation |
| Are all event consumers subscribed and caught up? | Not exposed. `JetStreamConsumer.health()` exists in-memory but nothing reads it over HTTP |
| Are projections fresh? | `ProjectionMetrics` emits Prometheus gauges but no pull API exists |
| Are outbox relays healthy? | No endpoint. Stale outbox rows are invisible except in Prometheus |
| What is DLQ depth, sync conflict count, push failures? | DLQ rows exist per module. No aggregated view. |

The result is that a vertical onboarding Fireproof ERP (or any future app) has no machine-readable way to confirm the platform is fully operational for its tenant. Support must manually SSH into each module and query tables. Alerts fire without diagnostic context.

---

## 2. Goals

1. **Provisioning verification** — single endpoint confirming all modules in the tenant's bundle have completed their `tenant.provisioned` handler.
2. **Operational vitals** — per-module snapshot of consumer health, outbox lag, DLQ depth, and projection freshness.
3. **Sync vitals** — conflict count, push failures, inflight attempts (integrations module only).
4. **Composable from existing data** — no new tables, no new persistent stores. Every number comes from a table or in-memory counter that already exists.
5. **No cross-module DB reads** — the aggregator fans out over HTTP to per-module endpoints. Module databases stay private.

---

## 3. Non-Goals

- No new UI or dashboard
- No Prometheus scraping changes
- No alerting rules (ops concern, separate bead)
- No cross-module DB reads (platform modular boundary is inviolable)
- No sync vitals for modules other than integrations (each module extends at its own pace)

---

## 4. Existing Data Sources (the full inventory)

Every number in this design pulls from a data source that already exists in the codebase. This table is the authoritative check against "new infrastructure" creep.

| Data | Table / Source | Location |
|---|---|---|
| Provisioning step completion | `provisioning_steps` | control-plane DB |
| Per-module provisioning status | `cp_tenant_module_status` | control-plane DB |
| Tenant's bundle modules | `cp_bundle_modules` JOIN `cp_tenant_bundle` | control-plane DB |
| Module service URLs | `cp_service_catalog` | control-plane DB |
| DLQ depth by failure kind | `event_dlq` | per-module DB |
| Outbox pending count + oldest age | `events_outbox` WHERE `published_at IS NULL` | per-module DB |
| Projection lag + cursor age | `projection_cursors` | per-module DB (modules using projections) |
| Consumer processed/skipped/DLQ counts | `JetStreamConsumer.health().snapshot()` | in-process `ConsumerHealth` atomics |
| Tenant readiness (received `tenant.provisioned`) | `TenantReadinessRegistry.is_ready(tenant_id)` | in-process registry |
| Sync conflict count | `integrations_sync_conflicts` WHERE `status='pending'` | integrations DB |
| Push failures last 24h | `integrations_sync_push_attempts` WHERE `status='failed'` | integrations DB |
| In-flight push attempts | `integrations_sync_push_attempts` WHERE `status='inflight'` | integrations DB |

---

## 5. Architecture Overview

Two-tier composable design. No new crates. No new tables.

```
Vertical / Operator
        │
        ▼
GET /api/control/tenants/{id}/vitals          (control-plane)
        │
        ├── reads provisioning_steps            (control-plane DB)
        ├── reads cp_tenant_module_status       (control-plane DB)
        ├── reads cp_bundle_modules             (control-plane DB)
        │
        └── HTTP fanout (parallel, 2s timeout)
              │
              ├── GET /api/vitals?tenant_id=X   (ar, gl, payments, ...)
              ├── GET /api/vitals?tenant_id=X   (subscriptions)
              ├── GET /api/vitals?tenant_id=X   (integrations)  ← adds sync stats
              └── ...
```

Each module's `/api/vitals` handler:
- Queries its own DB only (event_dlq, events_outbox, projection_cursors)
- Reads in-process `ConsumerHealth` snapshot
- Reads in-process `TenantReadinessRegistry`

---

## 6. Type Design (`platform/health` crate additions)

Add to `platform/health/src/lib.rs` (or a new `platform/health/src/vitals.rs`):

```rust
/// Depth of the dead-letter queue by failure class.
#[derive(Debug, Clone, Serialize)]
pub struct DlqVitals {
    pub total: u64,
    pub retryable: u64,
    pub fatal: u64,
    pub poison: u64,
}

/// Outbox relay health snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct OutboxVitals {
    /// Rows where published_at IS NULL.
    pub pending: u64,
    /// Age in seconds of the oldest unpublished row (None if queue is empty).
    pub oldest_pending_secs: Option<u64>,
}

/// Single projection cursor snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionVitals {
    pub name: String,
    pub tenant_id: String,
    /// Milliseconds between last processed event timestamp and now.
    pub lag_ms: i64,
    /// Seconds since cursor was last updated.
    pub age_seconds: i64,
}

/// Event consumer health snapshot (from in-process ConsumerHealth).
#[derive(Debug, Clone, Serialize)]
pub struct ConsumerVitals {
    pub name: String,
    pub processed: u64,
    pub skipped: u64,
    pub dlq: u64,
    pub running: bool,
}

/// Module-level vitals response (served at GET /api/vitals).
#[derive(Debug, Clone, Serialize)]
pub struct VitalsResponse {
    pub service_name: String,
    pub version: String,
    /// Whether this module has finished processing tenant.provisioned for this tenant.
    /// None if no tenant_id was supplied in the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_ready: Option<bool>,
    pub dlq: DlqVitals,
    pub outbox: OutboxVitals,
    /// Empty if the module does not use projections.
    pub projections: Vec<ProjectionVitals>,
    /// Empty if the module does not run JetStream consumers via JetStreamConsumer.
    pub consumers: Vec<ConsumerVitals>,
    /// Module-specific extended fields (e.g. sync stats for integrations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended: Option<serde_json::Value>,
    pub timestamp: String,
}
```

**Key invariants:**
- `VitalsResponse` is additive. New fields are `skip_serializing_if = "Option::is_none"` or default to empty Vec.
- Consumers that haven't upgraded still return a valid (partial) response.
- `extended` is a typed escape hatch for module-specific data (sync stats, etc.) without needing a new contract every time.

---

## 7. SDK Vitals Provider (`platform/platform-sdk` additions)

### 7.1 `VitalsProvider` trait

New file: `platform/platform-sdk/src/vitals.rs`

```rust
/// Trait for providing module-scoped operational vitals.
///
/// Implement this to expose data beyond what the standard provider collects.
/// The standard provider (StandardVitalsProvider) handles DLQ, outbox, and
/// projection_cursors automatically. Override this only for module-specific
/// extended data (e.g. sync stats in integrations).
#[async_trait::async_trait]
pub trait VitalsProvider: Send + Sync + 'static {
    /// Collect vitals for the given tenant (or all tenants if None).
    async fn collect_extended(
        &self,
        pool: &PgPool,
        tenant_id: Option<Uuid>,
    ) -> serde_json::Value;
}
```

### 7.2 `StandardVitalsProvider`

Built into the SDK. Queries:
- `event_dlq` → `DlqVitals` (4 SQL scalar queries via COUNT + GROUP BY failure_kind)
- Standard outbox table → `OutboxVitals` (COUNT WHERE published_at IS NULL + MIN(created_at))
- `projection_cursors` → `Vec<ProjectionVitals>` (SELECT all rows for tenant_id, compute lag/age)
- Registered `ConsumerHealth` handles → `Vec<ConsumerVitals>`
- `TenantReadinessRegistry` → `tenant_ready`

**Outbox table name** is read from the module manifest (`[events.publish].table` or defaults to `"events_outbox"`). If a module has no outbox, `OutboxVitals { pending: 0, oldest_pending_secs: None }`.

**No projection_cursors** for modules that don't use projections → empty Vec, not an error.

### 7.3 `ModuleBuilder` extension

```rust
impl ModuleBuilder {
    /// Register a vitals handler, wiring GET /api/vitals.
    ///
    /// StandardVitalsProvider is wired automatically. Call this with a custom
    /// provider only when you need extended data in the `extended` field.
    pub fn vitals_handler(self, extended: impl VitalsProvider) -> Self { ... }
}
```

The builder **always** wires the standard vitals handler at `GET /api/vitals`. The `extended` override is optional. If `.vitals_handler()` is never called, `/api/vitals` still works with standard data only.

### 7.4 Handler SQL

**DLQ depth:**
```sql
SELECT failure_kind, COUNT(*) as cnt
FROM event_dlq
GROUP BY failure_kind
```

**Outbox health:**
```sql
SELECT
  COUNT(*) FILTER (WHERE published_at IS NULL) AS pending,
  MIN(created_at) FILTER (WHERE published_at IS NULL) AS oldest_pending
FROM {outbox_table}
```

**Projection freshness (if tenant_id supplied):**
```sql
SELECT projection_name, tenant_id, last_event_occurred_at, updated_at
FROM projection_cursors
WHERE tenant_id = $1
```
Lag and age are computed in Rust from `Utc::now() - last_event_occurred_at` and `Utc::now() - updated_at`.

**Projection freshness (platform-wide, no tenant_id):**
```sql
SELECT projection_name, tenant_id, last_event_occurred_at, updated_at
FROM projection_cursors
ORDER BY updated_at ASC
LIMIT 50
```

### 7.5 Auth for `/api/vitals`

Same JWT gate as `/api/ready`. Platform-admin or service-level JWT required. The gate is enforced by the existing `security` middleware layer — no new auth logic.

---

## 8. Control-Plane Aggregator (`platform/control-plane` additions)

### 8.1 New handler: `GET /api/control/tenants/{id}/vitals`

New file: `platform/control-plane/src/handlers/tenant_vitals.rs`

**Response shape:**

```rust
pub struct TenantVitalsResponse {
    pub tenant_id: Uuid,
    pub tenant_status: String,
    pub provisioning: ProvisioningVitals,
    pub modules: Vec<ModuleVitalsEntry>,
    pub overall_healthy: bool,
    pub timestamp: String,
}

pub struct ProvisioningVitals {
    /// True only when every step in standard_provisioning_sequence() is `completed`.
    pub all_steps_complete: bool,
    pub steps: Vec<ProvisioningStepSummary>,
    /// Per-module provisioning status from cp_tenant_module_status.
    pub module_status: Vec<ModuleProvisioningStatus>,
}

pub struct ProvisioningStepSummary {
    pub step: String,
    pub order: i32,
    pub status: String,
}

pub struct ModuleProvisioningStatus {
    pub module_code: String,
    pub status: String,  // pending | active | error
}

pub struct ModuleVitalsEntry {
    pub module: String,
    /// None if the HTTP call to /api/vitals timed out or the module doesn't implement the endpoint yet.
    pub vitals: Option<health::VitalsResponse>,
    pub latency_ms: u64,
    pub error: Option<String>,
}
```

**Algorithm:**

```
1. Load tenant status from tenants table
2. Load provisioning_steps WHERE tenant_id = ?
3. Load cp_tenant_module_status WHERE tenant_id = ?
4. Load cp_bundle_modules JOIN cp_tenant_bundle WHERE tenant_id = ?  → module_codes
5. Load cp_service_catalog WHERE module_code IN (...)                → base_urls
6. Fan out in parallel: GET {base_url}/api/vitals?tenant_id={id} (2s timeout each)
7. Aggregate:
   overall_healthy = all_steps_complete
                   AND all modules tenant_ready == true
                   AND all modules dlq.total == 0
                   AND all modules outbox.pending == 0
8. Return TenantVitalsResponse
```

**Deliberate design choice: `overall_healthy` is strict.** A module with DLQ depth > 0 marks the tenant unhealthy. Verticals can read individual module entries for nuance. The strict flag is what machines (polling scripts, deployment gates) need.

**Backward compatibility:** If a module returns 404 or times out on `/api/vitals`, `ModuleVitalsEntry.vitals` is `None` and `error` explains why. `overall_healthy` does NOT count missing vitals as unhealthy — only `tenant_ready=false`, DLQ > 0, or outbox lag > 0 trigger unhealthy. This gives a migration window while modules add the endpoint.

Wait — actually, re-thinking the backward compat decision: we need to be careful here. If we silently ignore modules that haven't implemented `/api/vitals`, the endpoint gives false comfort. But if we mark them unhealthy, every currently-deployed module without the endpoint breaks.

**Resolution:** Add a `vitals_status` field per module: `"ok" | "degraded" | "unavailable"`. `overall_healthy` only counts modules that return a response. The caller sees per-module vitals_status and can decide. Add documentation that "unavailable" means the module hasn't yet implemented the endpoint.

### 8.2 Reuse existing fanout infrastructure

The `check_all_modules_ready` / `fetch_tenant_summary` pattern in `tenant-registry/src/summary.rs` and `tenant-registry/src/health.rs` already does the parallel HTTP fanout with 2s timeouts. The vitals aggregator reuses the same `reqwest::Client`, the same timeout constant (`MODULE_READINESS_TIMEOUT`), and the same `ModuleUrl` struct.

The only new code in the aggregator is:
1. The SQL to read provisioning state from control-plane DB (already demonstrated in `provisioning_status.rs`)
2. Deserializing `VitalsResponse` from each module's HTTP response
3. Building `TenantVitalsResponse`

This is ~120 lines of new Rust.

### 8.3 Routing

Add to control-plane router alongside existing provisioning endpoint:

```
GET /api/control/tenants/{id}/provisioning  (existing)
GET /api/control/tenants/{id}/vitals        (new)
```

---

## 9. Integrations Module Vitals Extension

The integrations module implements `VitalsProvider` with:

```rust
// platform/modules/integrations/src/vitals.rs

pub struct IntegrationsVitalsProvider;

#[async_trait]
impl VitalsProvider for IntegrationsVitalsProvider {
    async fn collect_extended(&self, pool: &PgPool, tenant_id: Option<Uuid>) -> serde_json::Value {
        // queries integrations_sync_conflicts + integrations_sync_push_attempts
        // returns: { sync_conflicts_pending, push_inflight, push_failures_24h }
    }
}
```

Wired in `main.rs`:
```rust
ModuleBuilder::from_manifest("module.toml")
    .vitals_handler(IntegrationsVitalsProvider)
    ...
    .run()
    .await
```

This is the only module-specific implementation. All other modules get standard vitals automatically.

---

## 10. OpenAPI Contract Extension

`contracts/control-plane/control-plane-v1.0.0.yaml` gets a new path:

```yaml
/api/control/tenants/{tenantId}/vitals:
  get:
    summary: Aggregated operational vitals for a tenant
    operationId: getTenantVitals
    tags: [monitoring]
    parameters:
      - name: tenantId
        in: path
        required: true
        schema:
          type: string
          format: uuid
    responses:
      '200':
        description: Vitals snapshot
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/TenantVitalsResponse'
      '404':
        description: Tenant not found
      '503':
        description: Control-plane not ready
```

Each module's OpenAPI spec (e.g. `contracts/ar/ar-v*.yaml`) gets a `/api/vitals` path. The SDK auto-generates or documents this — exact spec update is per-module and can be done as a follow-on bead.

---

## 11. `overall_healthy` Semantic Contract

This is the decision that will age. The contract is:

```
overall_healthy = true
  iff:
    all provisioning steps → completed
    AND all modules in bundle have vitals.tenant_ready = true
    AND all modules with vitals response have dlq.total = 0
    AND all modules with vitals response have outbox.pending = 0
    AND no module has vitals_status = "degraded"
```

Modules with `vitals_status = "unavailable"` (no /api/vitals endpoint yet) do NOT affect `overall_healthy`. This is documented explicitly in the response.

**Why this matters:** The `overall_healthy` flag is the deployment gate signal. Verticals poll this after provisioning to know when to let the first user in. False positives here are a security/integrity issue. False negatives are annoying but safe.

---

## 12. Verification Commands

These are the verification commands for beads that implement this plan:

```bash
# Phase 0 — Type additions to health crate
./scripts/cargo-slot.sh test -p health

# Phase 1 — SDK vitals handler
./scripts/cargo-slot.sh test -p platform-sdk

# Phase 2 — Control-plane aggregator
# Requires: control-plane DB running, at least ar + gl modules running
TENANT_REGISTRY_DATABASE_URL="postgres://..." \
CONTROL_PLANE_DATABASE_URL="postgres://..." \
./scripts/cargo-slot.sh test -p control-plane -- tenant_vitals

# Integration test: end-to-end vitals for a real tenant
curl -s http://localhost:8091/api/control/tenants/{TENANT_ID}/vitals | jq .

# Per-module vitals endpoint (after Phase 1 deployed)
curl -s http://localhost:8086/api/vitals | jq .   # ar
curl -s http://localhost:8090/api/vitals | jq .   # gl

# Confirm overall_healthy = true after provisioning a test tenant
curl -s "http://localhost:8091/api/control/tenants/{ID}/vitals" \
  | jq '.overall_healthy'
# → true
```

---

## 13. Phased Implementation (bead breakdown)

### Phase 0 — Health crate: new types (1 bead)
**Files:** `platform/health/src/vitals.rs`, `platform/health/src/lib.rs`
**What:** Add `VitalsResponse`, `DlqVitals`, `OutboxVitals`, `ProjectionVitals`, `ConsumerVitals` structs. Serde-derive. Unit tests for serialization shape.
**Verify:** `cargo test -p health`

### Phase 1 — SDK: `VitalsProvider` trait + `StandardVitalsProvider` + builder method (1 bead)
**Files:** `platform/platform-sdk/src/vitals.rs`, `platform/platform-sdk/src/builder.rs`
**What:** Implement `VitalsProvider` trait, `StandardVitalsProvider` (queries event_dlq, outbox, projection_cursors), wire `GET /api/vitals` route into `ModuleBuilder` (always-on, standard only unless overridden).
**Verify:** `cargo test -p platform-sdk -- vitals`

### Phase 2 — Control-plane aggregator (1 bead)
**Files:** `platform/control-plane/src/handlers/tenant_vitals.rs`, `platform/control-plane/src/handlers/mod.rs`, `platform/control-plane/src/lib.rs`
**What:** Add `GET /api/control/tenants/{id}/vitals` handler. Read provisioning from DB, fan out to `/api/vitals?tenant_id=X`, aggregate into `TenantVitalsResponse`.
**Verify:** Integration test against real DB + running ar/gl services.

### Phase 3 — Integrations extended vitals (1 bead)
**Files:** `modules/integrations/src/vitals.rs`, `modules/integrations/src/main.rs`
**What:** Implement `IntegrationsVitalsProvider` querying `integrations_sync_conflicts` + `integrations_sync_push_attempts`. Wire via `.vitals_handler(...)`.
**Verify:** `cargo test -p integrations -- vitals`

### Phase 4 — OpenAPI contract update (1 bead)
**Files:** `contracts/control-plane/control-plane-v1.0.0.yaml`
**What:** Add `/api/control/tenants/{id}/vitals` path + all component schemas.
**Verify:** `scripts/cargo-slot.sh run -p client-codegen` (or equivalent OpenAPI lint).

---

## 14. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Module has no `event_dlq` table (old module, pre-SDK-dlq) | StandardVitalsProvider catches `sqlx::Error::RowNotFound` / table-not-found, returns `DlqVitals { total: 0, ... }` with a warning log |
| Module has no `projection_cursors` table | Returns empty Vec, not an error |
| Module has no outbox table | Returns `OutboxVitals { pending: 0, oldest_pending_secs: None }` |
| Module times out on `/api/vitals` | `ModuleVitalsEntry.vitals = None`, `vitals_status = "unavailable"`, `overall_healthy` not affected |
| False healthy (DLQ was retried successfully mid-poll) | DLQ table persists entries even after replay — count is monotonically non-decreasing until explicit purge. Acceptable: cleared DLQ entries signal that problems were resolved. |
| Auth regression (platform_admin sees someone else's DLQ payload) | `/api/vitals` returns counts only, not payload contents. The DLQ payload stays in the DB. |
| `overall_healthy` definition drift | The semantic contract (Section 11) is pinned in the OpenAPI description and tested in integration tests. Changes require a plan revision and bead. |

---

## 15. How to Think About This (invariant)

> The vitals API is a **read-only aggregation surface over data that already exists**. It adds no new facts, no new state, no new persistence. Its only job is to make existing facts visible over HTTP without violating module DB boundaries. If any bead implementing this plan introduces a new table, a new NATS stream, or a new cross-module dependency, that bead is out of scope and should be split.

The guard-rail is: every SQL query in the plan must resolve to a table already shown in Section 4. Every in-process read must resolve to a struct already in the codebase. The aggregator is just routing — it borrows the `ModuleUrl` + `reqwest::Client` pattern from `tenant-registry/src/summary.rs` which already does exactly this for `/api/ready`.

---

## 16. Files Involved (by bead phase)

### Phase 0 (health crate types)
- `platform/health/src/vitals.rs` — new
- `platform/health/src/lib.rs` — add `pub mod vitals` + re-exports

### Phase 1 (SDK vitals provider)
- `platform/platform-sdk/src/vitals.rs` — new
- `platform/platform-sdk/src/builder.rs` — add `.vitals_handler()` method + auto-wire `/api/vitals`
- `platform/platform-sdk/src/lib.rs` — re-export `VitalsProvider`
- `platform/platform-sdk/tests/vitals.rs` — new integration test

### Phase 2 (control-plane aggregator)
- `platform/control-plane/src/handlers/tenant_vitals.rs` — new
- `platform/control-plane/src/handlers/mod.rs` — register handler
- `platform/control-plane/src/lib.rs` — register route

### Phase 3 (integrations extended)
- `modules/integrations/src/vitals.rs` — new
- `modules/integrations/src/main.rs` — wire vitals_handler

### Phase 4 (OpenAPI contract)
- `contracts/control-plane/control-plane-v1.0.0.yaml` — extend paths + schemas
