# Plug-and-Play Full Plan

**Date:** 2026-04-01
**Goal:** Every platform module is a true plug-and-play building block for verticals. A vertical developer adds client crates, sets env vars, and gets standard responses, reliable events, and automatic tenant/auth handling.

**Sources:** BrightHill 6-layer analysis, Claude Desktop verification (Layer 7), ChatGPT + Grok reviews, codebase verification.

---

## Workstream A: Quick Wins (no dependencies)

### A1. Correlation ID propagation in event consumers
`TracingContext::from_envelope()` exists at `event-bus/src/envelope/tracing_context.rs:94-111` but `wire_consumers()` at `consumer.rs:125-177` never calls it. ~10 lines to create a tracing span from the incoming envelope's correlation_id.

### A2. Inventory — finish response standardization
4 handlers missing utoipa annotations. 93% → 100%.

### A3. Party — finish response standardization
2 of 3 list endpoints use custom DataResponse instead of PaginatedResponse.

### A4. BOM — finish response standardization
3 handlers missing utoipa annotations. 89% → 100%.

### A5. AP — finish response standardization
`list_allocations` returns raw `Json(json!({ "allocations": records }))` instead of PaginatedResponse. 4 of 5 → 5 of 5.

---

## Workstream B: Event Bus Reliability

### B1. Event contract audit
Enumerate all subject mismatches across 16 publishing modules vs all consumers. AP double-prefix confirmed (`ap.events.ap.po_approved` vs expected `ap.po.approved`). Need complete mismatch inventory.

### B2. Fix event subject mismatches
Fix all mismatches found in B1. May be 1 bead or 3 depending on scope.

### B3. Wire provisioning outbox relay
Provisioning events are written to `provisioning_outbox` but no relay publishes them to NATS. The 7-step lifecycle is defined but never executed. Wire the relay so `provisioning_started` events actually arrive.

---

## Workstream C: Hot-Path Module Standardization

Each module needs: all list endpoints → PaginatedResponse<T>, all errors → ApiError, all handlers → utoipa annotations, openapi_dump complete.

### C1. GL response standardization
31 paths, partial utoipa. Close to done.

### C2. AR response standardization
99 endpoints. Largest module. Will need file splits if handlers exceed 500 LOC after changes.

### C3. Payments response standardization
Partial coverage. State machine handlers need ApiError migration.

### C4. Subscriptions response standardization
bill_run.rs is 334 LOC — standardize responses without SoC refactor.

### C5. Notifications response standardization
Already has SDK integration. Needs utoipa annotations + PaginatedResponse on list endpoints.

---

## Workstream D: Contract Publication

### D1. Maintenance OpenAPI spec
33 handlers across 11 files with zero utoipa annotations. openapi_dump binary exports empty spec. No generated client possible.

### D2. identity-auth OpenAPI + client generation
Module exists at `platform/identity-auth/` but has no openapi_dump binary and no generated client crate. Every vertical needing `register_user()` is on their own.

### D3. Consolidation clients — replace raw reqwest
3 Consolidation clients (`integrations/{gl,ar,ap}/client.rs`) are pure raw reqwest despite header comments claiming they wrap generated clients. Replace with actual generated client usage.

---

## Workstream E: Discovery

### E1. Event catalog generation
16 modules publish events. No way to discover what subjects exist, payload shapes, or source modules without reading code. Generate a catalog from code annotations or outbox table configs.

### E2. Service catalog endpoint
Control-plane endpoint returning `{module_name: url}` mappings. Only 3 inter-module HTTP URLs exist today (AR_BASE_URL, TENANT_REGISTRY_URL, DOC_MGMT_BASE_URL) so this is low urgency but completes the picture.

---

## Workstream F: TypeScript SDK

### F1. OpenAPI → TypeScript codegen pipeline
Set up `openapi-typescript` or similar to generate TS clients from module OpenAPI specs. CI step that regenerates on version bumps.

### F2. npm package structure
Package as `@7d/{module}-client`. Remove the 6 stray `.ts`/`.d.ts` files currently mixed into Rust client crates.

---

## Workstream G: Provisioning Orchestrator

### G1. Provisioning orchestrator design
The bundle schema is well-designed (cp_bundles, cp_bundle_modules, cp_tenant_bundle tables). Missing: async worker to execute the 7-step lifecycle, per-module status tracking, rollback on partial failure, SDK hook for vertical participation.

### G2. Provisioning orchestrator implementation
Build the async worker that reads bundles, sequences module provisioning, tracks status, and handles failures. Depends on G1 design.

### G3. SDK provisioning hook
`ModuleBuilder::on_tenant_provisioned(handler)` so verticals can participate in the provisioning cascade without manually subscribing to NATS subjects.

---

## Workstream H: Tenant Scoping

### H1. Centralize extract_tenant into SDK
`extract_tenant()` is copy-pasted across inventory, bom, party, notifications. Move to SDK as a standard extractor. Deduplicate across modules.

### H2. Tenant context middleware
Design decision: PostgreSQL session variables for RLS (`SET app.current_tenant`), or middleware that injects a TenantContext extractor. Either way, reduce the risk of forgotten `WHERE tenant_id = $N` clauses.

---

## Workstream I: SoC Cleanup

### I1. AR invoices.rs refactor
640 LOC, calls Party + GL + Subscriptions inline. Extract service layer.

### I2. Subscriptions bill_run.rs refactor
334 LOC single execute_bill_run function. Extract service layer.

### I3. AP match_engine.rs refactor
718 LOC, 3-way matching + DB + events in one function.

### I4. TTP billing.rs refactor
Reads env vars at request time (lines 100-103). Extract to startup config.

---

## Totals

| Workstream | Beads | Notes |
|------------|-------|-------|
| A: Quick wins | 5 | All independent, no deps |
| B: Event bus | 3 | B2 depends on B1 |
| C: Hot-path modules | 5 | Independent of each other |
| D: Contract publication | 3 | Independent |
| E: Discovery | 2 | Independent |
| F: TypeScript SDK | 2 | F2 depends on F1 |
| G: Provisioning | 3 | Sequential: G1 → G2 → G3 |
| H: Tenant scoping | 2 | H2 depends on H1 |
| I: SoC cleanup | 4 | Independent |
| **Total** | **29** | |
