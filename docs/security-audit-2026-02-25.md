# Security Audit Report — 2026-02-25 (Comprehensive)

## Executive Summary
This report summarizes the findings of a full security audit across all 16 platform modules, core platform services, and frontend applications. The platform has strong authentication foundations (RS256 JWT, JWKS, Key Rotation). A systemic tenant isolation vulnerability (C1) was identified across multiple modules and has been **largely remediated** — 9 of 13 affected modules are now resolved. RBAC enforcement (H1) is **fully resolved** across all 7 affected modules. The CORS wildcard warning (M2) for pdf-editor is **resolved**.

---

## CRITICAL FINDINGS

### C1. Systemic Tenant Isolation Bypass (CRITICAL)
**Severity: CRITICAL**
**Status: PARTIALLY RESOLVED — 9/13 modules fixed, 4 in progress**

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
| Fixed Assets | Path param `tenant_id` in `http/` handlers | IN PROGRESS | bd-24dc |
| Maintenance, Payments, PDF-Editor, Reporting | Query/body `tenant_id` in handlers | IN PROGRESS | bd-1qme |

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
| 2026-02-25 | C1 fix: Fixed Assets — in progress | bd-24dc |
| 2026-02-25 | C1 fix: Maintenance, Payments, PDF-Editor, Reporting — in progress | bd-1qme |

---

## Priority Action Items

1. **[P0 — IN PROGRESS]** Complete C1 tenant isolation fixes for Fixed Assets (bd-24dc), Maintenance, Payments, PDF-Editor, Reporting (bd-1qme).
2. **[P1 — PENDING]** Run verification sweep confirming all 18 modules derive tenant from VerifiedClaims (bd-ia5y).
3. **[P1 — PENDING]** E2E test proving spoofed headers are ignored when JWT is present (bd-3mwl).
4. **[P2 — PENDING]** Audit `platform/identity-auth` argon2 parameters against OWASP 2024 minimums (bd-1l2g).
5. **[P2 — OPEN]** Migrate service-to-service auth from symmetric HMAC to asymmetric RS256 (M1).
