# Security Audit Report — 2026-02-25 (Comprehensive)

## Executive Summary
This report summarizes the findings of a full security audit across all 18 platform modules, core platform services, and frontend applications. The platform has strong authentication foundations (RS256 JWT, JWKS, Key Rotation). A systemic tenant isolation vulnerability (C1) was identified across 18 modules and has been **fully remediated** — all 18 modules now derive tenant identity exclusively from JWT `VerifiedClaims`. RBAC enforcement (H1) is **fully resolved** across all 7 affected modules. The CORS wildcard warning (M2) for pdf-editor is **resolved**.

---

## CRITICAL FINDINGS

### C1. Systemic Tenant Isolation Bypass (CRITICAL)
**Severity: CRITICAL**
**Status: RESOLVED — all 18 modules fixed**

Multiple modules derived the tenant or application identity (`tenant_id` / `app_id`) from client-supplied headers (`X-Tenant-Id`, `X-App-Id`), query parameters, or request bodies without verifying them against the authenticated JWT claims.

**Impact:** An attacker with a valid token for Tenant A could access or modify data for Tenant B by simply spoofing the tenant identifier in the request.

**Remediation Status:**

| Module | Original Vector | Status | Bead |
|--------|----------------|--------|------|
| AP | `x-tenant-id` header in `http/vendors.rs` et al. | RESOLVED | bd-2koo |
| Party | `x-app-id` header in `http/party.rs` | RESOLVED | bd-354f |
| Integrations | `x-app-id` header in `http/external_refs.rs` | RESOLVED | bd-354f |
| Consolidation | `x-app-id` header in `http/config.rs` | RESOLVED | bd-354f |
| Timekeeping | `app_id` query param in `http/employees.rs` | RESOLVED | bd-354f |
| TTP | `tenant_id` query param in `http/metering.rs` | RESOLVED | bd-2koo |
| GL | `tenant_id` query param in `routes/gl_detail.rs` | RESOLVED | bd-2koo |
| Treasury | `X-App-Id` header in `http/` handlers | RESOLVED | bd-1ftb |
| AR (credit_notes) | `x-app-id` header in `routes/credit_notes.rs` | RESOLVED | bd-26f7 |
| Subscriptions | Client-supplied tenant in routes | RESOLVED | bd-2j5l |
| Inventory | Query param `tenant_id` in `http/` handlers | RESOLVED | bd-30mj |
| Fixed Assets | Path param `tenant_id` in `http/` handlers | RESOLVED | bd-24dc |
| Maintenance | Query/body `tenant_id` in handlers | RESOLVED | bd-1qme |
| Payments | Query/body `tenant_id` in handlers | RESOLVED | bd-1qme |
| PDF-Editor | Query/body `tenant_id` in handlers | RESOLVED | bd-1qme |
| Reporting | Query/body `tenant_id` in handlers | RESOLVED | bd-1qme |
| GL (sweep) | `tenant_id` query param in 11 route files | RESOLVED | bd-ia5y.1 |
| TTP (sweep) | `tenant_id` in billing.rs, service_agreements.rs | RESOLVED | bd-ia5y.2 |
| Consolidation (sweep) | `tenant_id` in consolidate.rs, intercompany.rs | RESOLVED | bd-ia5y.3 |
| Inventory routes (sweep) | `tenant_id` in items.rs, locations.rs, uom.rs | RESOLVED | bd-ia5y.4 |
| AR tax (sweep) | `app_id` in tax.rs JSON body | RESOLVED | bd-ia5y.5 |

All resolved modules now derive identity exclusively from `VerifiedClaims` extracted from JWT claims.

---

## HIGH SEVERITY FINDINGS

### H1. Missing RBAC Enforcement on Mutations (HIGH)
**Severity: HIGH**
**Status: RESOLVED**

All 7 affected modules now have `RequirePermissionsLayer` applied to mutation routes.

| Module | Status | Bead |
|--------|--------|------|
| integrations | RESOLVED | bd-1wxy |
| party | RESOLVED | bd-1wxy |
| ttp | RESOLVED | bd-1wxy |
| pdf-editor | RESOLVED | bd-1wxy |
| maintenance | RESOLVED | bd-f813 |
| notifications | RESOLVED | bd-f813 |
| payments | RESOLVED | bd-f813 |

---

## MEDIUM SEVERITY FINDINGS

### M1. Symmetric Service-to-Service Auth (MEDIUM)
**Severity: MEDIUM**
**Status: OPEN**
**Location:** `platform/security/src/service_auth.rs`

The platform uses HMAC-SHA256 with a shared `SERVICE_AUTH_SECRET` for internal calls.
**Risk:** Any compromised service with this secret can impersonate any other service.
**Remediation:** Migrate to asymmetric (RS256) signing for service tokens.

### M2. Missing CORS Wildcard Warning (MEDIUM)
**Severity: MEDIUM**
**Status: RESOLVED** (bd-kjgf)

The `pdf-editor` module now emits a warning when `CORS_ORIGINS` is set to `*` in non-development environments. A broader sweep (bd-kjgf) added the same warning to 10 additional modules.

---

## POSITIVE FINDINGS & VERIFICATIONS

### [SAFE] SQL Injection via Dynamic Table Names
**Location:** `platform/projections/src/admin.rs`
**Result:** Verified Safe. All dynamic interpolations are gated by `validate_projection_name` in `validate.rs` which uses a hardcoded allowlist and strict regex.

### [RESOLVED] Vulnerable NPM Dependencies
**Result:** Verified Resolved. Both `apps/tenant-control-plane-ui` and `apps/trashtech-pro` have been updated to `next@15.5.12`.

### [STRENGTH] Identity-Auth Architecture
The core identity service correctly implements RS256 signing, JWKS distribution, and supports zero-downtime key rotation through `JWT_PUBLIC_KEY_PREV`.

---

## Remediation Timeline

| Date | Action | Beads |
|------|--------|-------|
| 2026-02-25 | C1 fix: AP, GL, TTP tenant isolation | bd-2koo |
| 2026-02-25 | C1 fix: Party, Integrations, Consolidation, Timekeeping tenant isolation | bd-354f |
| 2026-02-25 | C1 fix: Treasury tenant isolation | bd-1ftb |
| 2026-02-25 | C1 fix: AR credit_notes tenant isolation | bd-26f7 |
| 2026-02-25 | C1 fix: Subscriptions tenant isolation | bd-2j5l |
| 2026-02-25 | C1 fix: Inventory tenant isolation | bd-30mj |
| 2026-02-25 | H1 fix: RBAC in Party, Integrations, TTP, pdf-editor | bd-1wxy |
| 2026-02-25 | H1 fix: RBAC in Maintenance, Notifications, Payments | bd-f813 |
| 2026-02-25 | M2 fix: CORS wildcard warning sweep (10 modules) | bd-kjgf |
| 2026-02-25 | C1 fix: Fixed Assets tenant isolation | bd-24dc |
| 2026-02-25 | C1 fix: Maintenance, Payments, PDF-Editor, Reporting tenant isolation | bd-1qme |
| 2026-02-25 | C1 verification sweep: found 5 additional modules with violations | bd-ia5y |
| 2026-02-25 | C1 fix: GL module (11 route files) — VerifiedClaims | bd-ia5y.1 |
| 2026-02-25 | C1 fix: TTP billing + service_agreements — VerifiedClaims | bd-ia5y.2 |
| 2026-02-25 | C1 fix: Consolidation (consolidate.rs, intercompany.rs) — VerifiedClaims | bd-ia5y.3 |
| 2026-02-25 | C1 fix: Inventory routes (items, locations, uom) — VerifiedClaims | bd-ia5y.4 |
| 2026-02-25 | C1 fix: AR tax.rs — VerifiedClaims | bd-ia5y.5 |

---

## Priority Action Items

1. **[P0 — RESOLVED]** C1 tenant isolation: all 18 modules now derive tenant from VerifiedClaims. Fixed Assets (bd-24dc), GL (bd-ia5y.1), TTP (bd-ia5y.2), Consolidation (bd-ia5y.3), Inventory routes (bd-ia5y.4), AR tax (bd-ia5y.5).
2. **[P1 — RESOLVED]** Verification sweep confirming all 18 modules derive tenant from VerifiedClaims (bd-ia5y). Sweep report: `docs/c1-verification-sweep-2026-02-25.md`.
3. **[P1 — PENDING]** E2E test proving spoofed headers are ignored when JWT is present (bd-3mwl).
4. **[P2 — RESOLVED]** Audit `platform/identity-auth` argon2 parameters against OWASP 2024 minimums (bd-1l2g).
5. **[P2 — OPEN]** Migrate service-to-service auth from symmetric HMAC to asymmetric RS256 (M1).
