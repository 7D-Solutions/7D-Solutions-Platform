# API Consistency Audit — All Platform Services

**Bead:** bd-2ou15
**Date:** 2026-03-24
**Auditor:** PurpleCliff
**Scope:** 25 Rust microservices in `modules/`

---

## Executive Summary

The platform has reasonable consistency in error response format and health endpoints. The two most urgent findings are:

1. **Production service has no permission checks** — any valid JWT can mutate production data (workcenters, routings, work orders, etc.).
2. **Production response ID fields are non-standard** — workcenters return `workcenter_id`, routings return `routing_template_id`; all other services return `id`.

Secondary findings: list response envelopes are inconsistent (some paginated, some bare arrays), tenant_id is required in request bodies for inventory and production but extracted from JWT in all other services, and several permission name strings use underscores while others don't.

---

## 1. Routes by Service

| Service | Module Path | Key Mutation Routes | Key Read Routes |
|---------|-------------|---------------------|-----------------|
| **AP** | `modules/ap` | POST /api/ap/vendors, POST /api/ap/pos, POST /api/ap/bills, POST /api/ap/payment-runs | GET /api/ap/vendors, GET /api/ap/bills, GET /api/ap/payment-terms |
| **AR** | `modules/ar` | POST /api/ar/customers, POST /api/ar/invoices, POST /api/ar/charges, POST /api/ar/refunds, POST /api/ar/subscriptions | GET /api/ar/customers, GET /api/ar/invoices, GET /api/ar/charges |
| **BOM** | `modules/bom` | POST /api/bom, POST /api/bom/{id}/revisions, POST /api/bom/revisions/{id}/lines, POST /api/eco | GET /api/bom/{id}, GET /api/bom/by-part/{part_id}, GET /api/bom/{id}/explosion |
| **Consolidation** | `modules/consolidation` | POST /api/consolidation/groups, POST /api/consolidation/groups/{id}/consolidate | GET /api/consolidation/groups/{id}/trial-balance |
| **Customer Portal** | `modules/customer-portal` | POST /portal/auth/login, POST /portal/auth/logout, POST /portal/admin/users | GET /portal/me, GET /portal/docs, GET /portal/status/feed |
| **Fixed Assets** | `modules/fixed-assets` | POST /api/fixed-assets/assets, POST /api/fixed-assets/depreciation/runs | GET /api/fixed-assets/assets, GET /api/fixed-assets/disposals |
| **GL** | `modules/gl` | POST /api/gl/accounts, POST /api/gl/fx-rates, POST /api/gl/revrec/contracts, POST /api/gl/accruals/templates | GET /api/gl/trial-balance, GET /api/gl/balance-sheet, GET /api/gl/income-statement |
| **Integrations** | `modules/integrations` | POST /api/integrations/external-refs, POST /api/integrations/connectors | GET /api/integrations/external-refs/by-entity, GET /api/integrations/connectors |
| **Inventory** | `modules/inventory` | POST /api/inventory/items, POST /api/inventory/receipts, POST /api/inventory/issues, POST /api/inventory/uoms, POST /api/inventory/adjustments | GET /api/inventory/items (paginated), GET /api/inventory/items/{id} |
| **Maintenance** | `modules/maintenance` | POST /api/maintenance/assets, PATCH /api/maintenance/assets/{id}, POST /api/maintenance/work-orders | GET /api/maintenance/assets, GET /api/maintenance/work-orders |
| **Notifications** | `modules/notifications` | POST /api/notifications/templates, POST /api/notifications/sends | GET /api/notifications/templates, GET /api/notifications/inbox |
| **Numbering** | `modules/numbering` | POST /allocate, POST /confirm, PUT /api/numbering/policy/{module} | GET /api/numbering/policy/{module} |
| **Party** | `modules/party` | POST /api/party/companies, POST /api/party/individuals, POST /api/party/parties/{id}/contacts | GET /api/party/parties, GET /api/party/parties/search |
| **Payments** | `modules/payments` | POST /api/payments/checkout-sessions | GET /api/payments/checkout-sessions/{id} |
| **PDF Editor** | `modules/pdf-editor` | POST /api/pdf/forms/templates, POST /api/pdf/forms/submissions | GET /api/pdf/forms/templates, GET /api/pdf/forms/submissions |
| **Production** | `modules/production` | POST /api/production/workcenters, POST /api/production/routings, POST /api/production/work-orders, POST /api/production/time-entries/start | GET /api/production/workcenters, GET /api/production/routings, GET /api/production/routings/by-item |
| **Quality Inspection** | `modules/quality-inspection` | POST /api/quality-inspection/plans, POST /api/quality-inspection/inspections | GET /api/quality-inspection/inspections/{id}, GET /api/quality-inspection/inspections/by-part-rev |
| **Reporting** | `modules/reporting` | POST /api/reporting/rebuild | GET /api/reporting/pl, GET /api/reporting/balance-sheet, GET /api/reporting/kpis |
| **Shipping & Receiving** | `modules/shipping-receiving` | (via build_mutation_router) POST /api/shipping-receiving/... | GET /api/shipping-receiving/... |
| **Subscriptions** | `modules/subscriptions` | (via subscriptions_router) POST /api/subscriptions/... | GET /api/subscriptions/... |
| **Timekeeping** | `modules/timekeeping` | POST /api/timekeeping/employees, POST /api/timekeeping/entries, POST /api/timekeeping/rates | GET /api/timekeeping/entries, GET /api/timekeeping/approvals |
| **Treasury** | `modules/treasury` | POST /api/treasury/accounts/bank, POST /api/treasury/recon/auto-match, POST /api/treasury/statements/import | GET /api/treasury/accounts, GET /api/treasury/cash-position |
| **TTP** | `modules/ttp` | POST /api/ttp/billing-runs, POST /api/metering/events | GET /api/metering/trace, GET /api/ttp/service-agreements |
| **Workflow** | `modules/workflow` | POST /api/workflow/definitions, POST /api/workflow/instances | GET /api/workflow/definitions, GET /api/workflow/instances/{id} |
| **Workforce Competence** | `modules/workforce-competence` | POST /api/workforce-competence/artifacts, POST /api/workforce-competence/assignments | GET /api/workforce-competence/artifacts/{id}, GET /api/workforce-competence/authorization |

---

## 2. Response ID Field Name (Create Endpoints)

The standard is `id`. All services return `id` except:

| Service | Field Name | Expected |
|---------|-----------|----------|
| Production — workcenters | `workcenter_id` | `id` |
| Production — routings | `routing_template_id` | `id` |

**Action required:** production domain structs `Workcenter` and `RoutingTemplate` use non-standard primary key field names. This forces callers to handle production differently from every other service.

---

## 3. List / Search Response Envelope Format

No platform-wide standard exists. Services fall into two camps:

**Paginated envelope** `{items: [], total, limit, offset}`:
- Inventory

**Bare array** `[]`:
- AR (returns `Vec<Invoice>`)
- Party (returns `Vec<Party>`)
- Most other services (not fully verified, but the pattern is common)

**Recommended standard:** `{items: [], total: N, limit: N, offset: N}` for all collection endpoints to enable consistent pagination.

---

## 4. Error Response Format

**Consistent format** (used by most services):
```json
{"error": "error_code_snake_case", "message": "Human-readable description"}
```

**Deviations:**
- **GL** (`accounts.rs`): Uses `AccountErrorResponse` struct with `status` (StatusCode) and `message` fields — different shape from the platform standard.
- **Production**: Some error variants return the same `{"error": ..., "message": ...}` shape, but 409 conflict responses do not include the existing resource UUID (see §6).

---

## 5. Health Check Endpoints

**Standard** (22 of 25 services):
- `GET /healthz` — liveness probe (Kubernetes/Docker)
- `GET /api/health` — liveness
- `GET /api/ready` — readiness
- `GET /api/version` — version info
- `GET /metrics` — Prometheus

**Deviations:**

| Service | `/healthz` | `/api/health` | `/api/ready` | `/api/version` | Notes |
|---------|-----------|--------------|-------------|---------------|-------|
| customer-portal | ❌ | ✅ | ✅ | ✅ | Uses `/portal/` prefix for app routes; missing `/healthz` |
| workforce-competence | ❌ | ✅ | ✅ | ✅ | Missing `/healthz`; has extra `/api/schema-version` |
| notifications | ✅ | ✅ | ✅ | ✅ | Also exposes `/ready` (duplicate, unguarded path) |
| numbering | ✅ | ✅ | ✅ | ✅ | Extra `/api/schema-version` |

---

## 6. Auth Requirements

**Standard pattern:**
```
outer layer: optional_claims_mw (extracts tenant from JWT if present)
mutation routes: .route_layer(RequirePermissionsLayer::new(&[module.mutate]))
read routes: optional_claims_mw only (tenant enforced inside handler)
```

**Deviations:**

| Service | Pattern | Issue |
|---------|---------|-------|
| **Production** | `optional_claims_mw` ONLY | ⚠️ **No permission checks on any route.** Any request with a valid JWT (even read-only JWTs) can create/modify workcenters, routings, work orders, time entries, etc. |
| Customer Portal | Custom `PortalJwt` | Intentional — separate B2B portal auth system, not platform JWT |

**Production auth gap is the most critical finding in this audit.** File: `modules/production/src/main.rs` — no import of `RequirePermissionsLayer` or `permissions::*`.

Note: There is no `PRODUCTION_MUTATE` permission constant defined in `platform/security/src/permissions.rs`. The permission constant needs to be added alongside fixing the middleware.

---

## 7. Permission Name Conventions

**Naming convention:** `<module>.<action>` where action is `mutate` or `read`.

**Consistent** (single-word module names):
```
ar.mutate, ar.read
payments.mutate, payments.read
subscriptions.mutate
notifications.mutate, notifications.read
maintenance.mutate, maintenance.read
inventory.mutate, inventory.read
reporting.mutate, reporting.read
treasury.mutate, treasury.read
ap.mutate, ap.read
consolidation.mutate, consolidation.read
timekeeping.mutate, timekeeping.read
party.mutate, party.read
integrations.mutate, integrations.read
ttp.mutate, ttp.read
workflow.mutate, workflow.read
bom.mutate, bom.read
```

**Inconsistent — underscores in module name** (vs single-word pattern):
```
fixed_assets.mutate, fixed_assets.read
pdf_editor.mutate, pdf_editor.read
shipping_receiving.mutate, shipping_receiving.read
workforce_competence.mutate, workforce_competence.read
quality_inspection.mutate, quality_inspection.read
```

**Non-standard action names:**
```
gl.post   (action is "post" not "mutate" — GL convention, probably intentional)
gl.read
numbering.allocate  (action is "allocate" not "mutate")
numbering.read
```

**Missing entirely:**
- `production` — no permission constant defined (production service has no auth; see §6)
- `customer_portal` — uses custom PortalJwt system, no platform permission

---

## 8. Tenant-ID Source

**Extract from JWT claims only** (standard, preferred):
GL, BOM, AR, AP, Party, Payments, Subscriptions, Treasury, Maintenance, Notifications, Numbering, Reporting, Timekeeping, TTP, Fixed Assets, Integrations, Consolidation, Workflow, Workforce Competence, Quality Inspection, Shipping & Receiving, PDF Editor

**Require `tenant_id` in request body** (non-standard):
- **Inventory** — `CreateItemRequest.tenant_id: String` (required)
- **Production** — `CreateWorkcenterRequest.tenant_id: String`, `CreateRoutingRequest` likely same

This means callers of inventory and production must know and supply the tenant ID explicitly, while every other service derives it from the JWT. This creates extra client burden and is an inconsistency.

---

## 9. 409 Conflict Response Body

None of the services return the existing resource's UUID in a 409 response. Callers must make an additional GET request to retrieve the existing resource ID on conflict. This forces extra round-trips for idempotent seeding/import workflows.

**Current 409 patterns:**

| Service | 409 Body |
|---------|----------|
| Inventory (duplicate SKU) | `{"error": "duplicate_sku", "message": "SKU 'X' already exists for tenant 'Y'"}` |
| Production (duplicate workcenter code) | `{"error": "duplicate_code", "message": "Workcenter code 'X' already exists for tenant 'Y'"}` |
| GL (duplicate account code) | `AccountErrorResponse { status: CONFLICT, message: "..." }` |
| BOM (duplicate part_id) | `{"error": "conflict", "message": "..."}` |

**Recommended:** Include `existing_id: UUID` in 409 bodies to eliminate GET-after-409 patterns.

---

## Findings Summary

| # | Severity | Finding | Affected Services |
|---|----------|---------|-------------------|
| F1 | **Critical** | Production has no permission checks | production |
| F2 | **Critical** | No `PRODUCTION_MUTATE` permission constant defined | platform/security |
| F3 | **High** | Production response ID fields are non-standard (`workcenter_id`, `routing_template_id`) | production |
| F4 | **High** | Tenant-ID required in request body instead of from JWT | inventory, production |
| F5 | **Medium** | List response envelopes inconsistent (paginated vs bare array) | inventory vs all others |
| F6 | **Medium** | Missing `/healthz` liveness probe | customer-portal, workforce-competence |
| F7 | **Medium** | GL error response shape differs from platform standard | gl |
| F8 | **Low** | Permission name inconsistency (underscores vs single-word module names) | fixed_assets, pdf_editor, shipping_receiving, workforce_competence, quality_inspection |
| F9 | **Low** | Non-standard permission action names (`gl.post`, `numbering.allocate`) | gl, numbering |
| F10 | **Low** | 409 conflict responses do not include existing resource UUID | all services |
| F11 | **Low** | Notifications exposes duplicate `/ready` route | notifications |

---

## Recommended Standardization

### Priority 1 — Fix immediately

1. **Add `PRODUCTION_MUTATE` permission to `platform/security/src/permissions.rs`**
2. **Add `RequirePermissionsLayer::new(&[permissions::PRODUCTION_MUTATE])` to all production mutation routes**

### Priority 2 — Fix before external API consumers

3. **Rename `workcenter_id` → `id` and `routing_template_id` → `id`** in production domain structs, update DB column aliases as needed
4. **Remove `tenant_id` from `CreateItemRequest` and `CreateWorkcenterRequest`** — extract from JWT inside handler
5. **Adopt standard list envelope** `{items: [], total, limit, offset}` — currently only inventory does this; others should match

### Priority 3 — Operational

6. **Add `/healthz` to customer-portal and workforce-competence**
7. **Standardize GL error response** to `{"error": "...", "message": "..."}` shape

### Priority 4 — Nice-to-have

8. **Return `existing_id` in 409 conflict bodies** to eliminate GET-after-409 round-trips
9. **Consider renaming** `fixed_assets`, `pdf_editor`, `shipping_receiving`, `workforce_competence`, `quality_inspection` permissions to drop underscores (breaking change — coordinate with auth service)
