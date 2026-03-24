# Security Audit: Tenant Isolation and Auth Enforcement

**Date:** 2026-03-24
**Bead:** bd-29c9i
**Auditor:** MaroonHarbor
**Scope:** All 25 service modules + platform/security
**Customer context:** Aerospace/Defense, ITAR data

---

## Methodology

Six audit dimensions per the bead acceptance criteria:

1. **Auth enforcement** — mutation routes have `RequirePermissionsLayer`
2. **Tenant isolation** — all SQL queries scope to JWT `tenant_id`
3. **JWT claim extraction** — handlers use `VerifiedClaims`, not request body
4. **Cross-tenant leakage** — valid JWT for tenant A cannot access tenant B data
5. **Error information leakage** — 404/409 do not reveal resource existence
6. **RBAC completeness** — permission constants defined and used consistently

---

## Findings Summary

| Severity | Count | Findings |
|----------|-------|---------|
| Critical | 0 | — |
| High     | 2 | H1, H2 |
| Medium   | 3 | M1, M2, M3 |
| Low      | 2 | L1, L2 |

---

## High Severity

### H1 — Production Module: No Permission Enforcement on Mutation Routes

**File:** `modules/production/src/main.rs`
**Affected routes:** All 29 POST/PUT routes in the production module

The production module uses `optional_claims_mw` to extract claims but applies no `RequirePermissionsLayer` to any route. All other 24 modules gate mutation routes with `RequirePermissionsLayer`. Production is the sole exception.

Any caller with a valid platform JWT — regardless of their permissions — can:
- Create, release, and close work orders
- Create, update, and deactivate workcenters
- Start/stop time entries
- Add routing steps, start/complete operations
- Post finished-goods receipts and component issue events
- Start and end workcenter downtime records

**Evidence:** `RequirePermissionsLayer` grep across all module `src/` directories returns every module except `production`.

**Tenant isolation is intact at the SQL layer** — every domain function filters by `tenant_id` extracted from JWT. Cross-tenant access is not possible. But any authenticated user with any permission can write to production data within their tenant.

**Fix:** Add `RequirePermissionsLayer::new(&[permissions::PRODUCTION_MUTATE])` to mutation routes and `RequirePermissionsLayer::new(&[permissions::PRODUCTION_READ])` to read routes. Also see H2.

---

### H2 — Production Permission Constants Missing from Central Registry

**File:** `platform/security/src/permissions.rs`
**Missing:** `PRODUCTION_MUTATE`, `PRODUCTION_READ`

The platform security library defines permission constants for all 24 other modules. The production module has no entries. This is likely why H1 exists — there were no constants to reference when mounting the permission layer.

All 24 other module permission constants are present in the central registry, including modules added later in the build (BOM, Workforce Competence, Quality Inspection, Shipping/Receiving, etc.).

**Fix:** Add to `permissions.rs`:
```rust
// ── Production ─────────────────────────────────────────────────────────

pub const PRODUCTION_MUTATE: &str = "production.mutate";
pub const PRODUCTION_READ: &str = "production.read";
```
Then apply them in `modules/production/src/main.rs` (resolves H1).

---

## Medium Severity

### M1 — BOM: Raw PostgreSQL Error Message Exposed in 409 Response

**File:** `modules/bom/src/http/bom_routes.rs:60`

When a unique constraint violation occurs (PostgreSQL error code `23505`), the BOM module returns the raw database error message directly in the HTTP response:

```rust
BomError::Database(ref e) => {
    if let sqlx::Error::Database(dbe) = e {
        if dbe.code().as_deref() == Some("23505") {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "duplicate", "message": dbe.message() })),
            );
        }
    }
}
```

PostgreSQL constraint violation messages include the constraint name, e.g.:
> `duplicate key value violates unique constraint "bom_bom_headers_tenant_id_part_id_key"`

This exposes:
- Internal database table names (`bom_bom_headers`)
- Column names that form the constraint
- The fact that a specific key combination already exists (timing-attack vector for resource enumeration)

**Fix:** Replace `dbe.message()` with a generic message:
```rust
Json(json!({ "error": "duplicate", "message": "A BOM record with this key already exists" }))
```

---

### M2 — Read Routes Unprotected by Permission Check (ITAR Risk)

**Affected modules:** AR, GL, Maintenance, Notifications, Payments, Subscriptions, Timekeeping, Treasury, TTP, Workflow, Integrations, Consolidation, Reporting, Party (reads), Production (all routes)

Most modules only apply `RequirePermissionsLayer` to mutation routes. Read routes require a valid JWT (authentication) but no specific read permission (authorization). Any platform user with a valid JWT can read financial, inventory, and production data within their tenant.

For an ITAR aerospace/defense customer, uncontrolled read access to:
- GL journal entries and trial balances
- AR invoices
- Production work orders and routings
- Reporting dashboards

...represents a data leakage risk even within a tenant, if different user roles should have scoped access.

**Modules with READ permission enforcement (best practice):** AP, BOM, Inventory.

**Note:** This is consistent with the comment in `permissions.rs` — "read — query-only (reserved; not yet enforced by default)" — so this is a known design decision, not a bug. Escalating to medium given the ITAR context.

**Fix:** For ITAR compliance, enforce read permissions on modules handling classified or export-controlled data. Priority order: Reporting, GL, Production (pending H1 fix), AR, Inventory.

---

### M3 — Customer Portal Admin Routes Use `party.mutate` Permission

**File:** `modules/customer-portal/src/lib.rs:54`

Customer portal admin operations (invite user, link documents, manage status cards) are gated with `RequirePermissionsLayer::new(&[permissions::PARTY_MUTATE])`. This means any user with `party.mutate` permission can also administer portal users — two unrelated capabilities sharing one permission string.

```rust
let admin_routes = Router::new()
    .route("/portal/admin/users", post(http::admin::invite_user))
    .route("/portal/admin/docs/link", ...)
    .route("/portal/admin/status-cards", ...)
    .route_layer(RequirePermissionsLayer::new(&[permissions::PARTY_MUTATE]))
```

An operator who can create/update party records (companies, individuals) should not automatically have the ability to invite users to the customer portal. These are distinct privileges.

**Fix:** Add `CUSTOMER_PORTAL_ADMIN: &str = "customer_portal.admin"` to `permissions.rs` and update the route layer. This requires token issuance changes to include the new permission for portal administrators.

---

## Low Severity

### L1 — Party: No Unique Constraint on Company Display Name per Tenant

**File:** `modules/party/db/migrations/20260219000001_create_party_schema.sql`

The `party_parties` table has no `UNIQUE(app_id, display_name)` constraint. Multiple company records with identical display names can be created within the same tenant. This can cause:
- Duplicate vendor/customer entries in downstream modules (AP, AR)
- Confusion in search results
- Potential for data integrity issues in matching workflows

The bead description identified this as a known gap.

**Fix:** Add a partial unique index or constraint for company-type parties:
```sql
CREATE UNIQUE INDEX idx_party_companies_display_name_unique
    ON party_parties(app_id, lower(display_name))
    WHERE party_type = 'company' AND status != 'archived';
```
Note: case-insensitive comparison prevents near-duplicate names like "Acme Corp" / "acme corp".

---

### L2 — Party Module: `app_id` Column Stores JWT `tenant_id`

**Files:** `modules/party/db/migrations/20260219000001_create_party_schema.sql`, `modules/party/src/http/party.rs`

The party module isolates data using a column named `app_id`, but this column stores the JWT `tenant_id` value:

```rust
// handler extracts tenant_id from JWT:
Some(Extension(c)) => Ok(c.tenant_id.to_string()),
// ...then passes it as `app_id` to service functions
let app_id = extract_tenant(&claims)?;
service::create_company(&state.pool, &app_id, ...)
```

The JWT `VerifiedClaims` struct has both `tenant_id: Uuid` AND `app_id: Option<Uuid>` fields. The party module currently ignores the JWT `app_id` and uses `tenant_id` for all isolation. The naming mismatch — using a column called `app_id` to store `tenant_id` — could cause future developers to assume the wrong JWT field is used.

**Fix (optional):** No functional change needed. Consider renaming the `app_id` column to `tenant_id` in a future migration to align with platform conventions, or adding an inline comment in `extract_tenant` clarifying that `app_id` stores the JWT `tenant_id`.

---

## Non-Findings (Verified Secure)

These items from the bead description were investigated and found to be correctly implemented:

### Tenant isolation in SQL queries ✅
All 25 modules consistently include `AND tenant_id = $N` (or equivalent `app_id`) in SQL queries for tenant-scoped data. Spot-checked: GL, AR, AP, BOM, Production, Inventory, Maintenance, Shipping/Receiving, Party. No SQL query was found that selects tenant data without a tenant filter.

### JWT claim extraction in handlers ✅
All modules extract `tenant_id` from `VerifiedClaims` (populated by `optional_claims_mw` from the JWT). No handler was found that reads `tenant_id` from the HTTP request body and uses it without validation against JWT claims. The GL accruals and revrec handlers correctly overwrite body `tenant_id` with the JWT-extracted value.

### Cross-tenant leakage ✅
Resources are always fetched with both the resource ID and the JWT tenant_id as SQL parameters. No handler fetches a resource by ID alone without also filtering by tenant. Tested: `WorkOrderRepo::find_by_id` requires both `work_order_id` and `tenant_id`; `party_parties` WHERE clause always includes `app_id`.

### AR permission layer ✅
The AR module now correctly has `RequirePermissionsLayer::new(&[permissions::AR_MUTATE])` on all mutation routes (the bead notes this was previously missing — it has since been added).

### RBAC constants completeness ✅ (with H2 exception)
24 of 25 modules have permission constants in `platform/security/src/permissions.rs`. Only production is missing (H2).

---

## Child Beads Required

| Finding | Child Bead | Priority |
|---------|-----------|----------|
| H1 — Production no permission enforcement | Create bead | P1 |
| H2 — Production permissions missing from registry | Same bead as H1 | P1 |
| M1 — BOM raw DB message leak | Create bead | P2 |
| M3 — Customer portal uses wrong permission | Create bead | P2 |

M2 (read route enforcement) and L1/L2 are recorded here for awareness. M2 should be discussed with BrightHill to determine ITAR compliance scope before creating beads.

---

## Appendix: Module Auth Coverage Matrix

| Module | RequirePermissionsLayer | Mutations Protected | Reads Protected |
|--------|------------------------|---------------------|-----------------|
| ap | ✅ | ✅ AP_MUTATE | ✅ AP_READ |
| ar | ✅ | ✅ AR_MUTATE | ❌ open (auth only) |
| bom | ✅ | ✅ BOM_MUTATE | ✅ BOM_READ |
| consolidation | ✅ | ✅ CONSOLIDATION_MUTATE | ❌ open (auth only) |
| customer-portal | ✅ | ✅ PARTY_MUTATE (see M3) | portal JWT |
| fixed-assets | ✅ | ✅ FIXED_ASSETS_MUTATE | ❌ open (auth only) |
| gl | ✅ | ✅ GL_POST | ❌ open (auth only) |
| integrations | ✅ | ✅ INTEGRATIONS_MUTATE | ❌ open (auth only) |
| inventory | ✅ | ✅ INVENTORY_MUTATE | ✅ INVENTORY_READ |
| maintenance | ✅ | ✅ MAINTENANCE_MUTATE | ❌ open (auth only) |
| notifications | ✅ | ✅ NOTIFICATIONS_MUTATE | ❌ open (auth only) |
| numbering | ✅ | ✅ NUMBERING_ALLOCATE | ❌ open (auth only) |
| party | ✅ | ✅ PARTY_MUTATE | ❌ open (auth only) |
| payments | ✅ | ✅ PAYMENTS_MUTATE | ❌ open (auth only) |
| pdf-editor | ✅ | ✅ PDF_EDITOR_MUTATE | ❌ open (auth only) |
| **production** | **❌** | **❌ NONE (H1, H2)** | **❌ NONE** |
| quality-inspection | ✅ | ✅ QUALITY_INSPECTION_MUTATE | ❌ open (auth only) |
| reporting | ✅ | ✅ REPORTING_MUTATE | ❌ open (auth only) |
| shipping-receiving | ✅ | ✅ SHIPPING_RECEIVING_MUTATE | ❌ open (auth only) |
| subscriptions | ✅ | ✅ SUBSCRIPTIONS_MUTATE | ❌ open (auth only) |
| timekeeping | ✅ | ✅ TIMEKEEPING_MUTATE | ❌ open (auth only) |
| treasury | ✅ | ✅ TREASURY_MUTATE | ❌ open (auth only) |
| ttp | ✅ | ✅ TTP_MUTATE | ❌ open (auth only) |
| workflow | ✅ | ✅ WORKFLOW_MUTATE | ❌ open (auth only) |
| workforce-competence | ✅ | ✅ WORKFORCE_COMPETENCE_MUTATE | ❌ open (auth only) |
