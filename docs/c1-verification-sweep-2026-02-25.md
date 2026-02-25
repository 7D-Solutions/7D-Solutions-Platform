# C1 Verification Sweep: Tenant Derivation from VerifiedClaims

**Date:** 2026-02-25
**Bead:** bd-ia5y
**Auditor:** MaroonHarbor (automated sweep)

## Summary

Swept all 18 modules for HTTP handlers that derive tenant identity from
client-supplied input (query params, path params, request body, headers)
instead of JWT `VerifiedClaims`.

**Result: 13/18 modules PASS. 5 modules have remaining C1 violations.**

---

## Anti-patterns searched

| Pattern | Description |
|---------|-------------|
| `x-app-id` / `x-tenant-id` header extraction | Should be zero in handlers |
| Query param `tenant_id` / `app_id` in handler structs | Should be zero (except event/consumer) |
| Path param `tenant_id` extraction | Should be zero |
| Hardcoded `"default"` tenant strings | Should be zero |

**Exclusions (per bead instructions):**
- Event envelope / consumer code (tenant_id in outbox messages is expected)
- Outbound HTTP client calls (setting headers on outgoing requests)
- Test files
- Admin endpoints gated by `X-Admin-Token` (not tenant-facing)

---

## Per-Module Results

### PASS (13 modules)

| # | Module | Notes |
|---|--------|-------|
| 1 | AP | All handlers use `VerifiedClaims` + `extract_tenant` |
| 2 | Fixed Assets | `VerifiedClaims` + `extract_tenant` helper in `helpers/tenant.rs` |
| 3 | Integrations | `VerifiedClaims` used; webhook extracts app_id from JWT |
| 4 | Maintenance | All handlers use `VerifiedClaims` + `extract_tenant` |
| 5 | Notifications | N/A — event-driven only; no tenant-scoped HTTP API |
| 6 | Party | HTTP handlers use `VerifiedClaims` |
| 7 | Payments | Uses `VerifiedClaims` + `extract_tenant` |
| 8 | PDF Editor | Uses `VerifiedClaims` + `extract_tenant` |
| 9 | Reporting | Non-admin handlers use `VerifiedClaims` |
| 10 | Shipping-Receiving | Uses `VerifiedClaims` |
| 11 | Subscriptions | Uses `VerifiedClaims` in routes |
| 12 | Timekeeping | Uses `VerifiedClaims` |
| 13 | Treasury | Uses `VerifiedClaims` |

### FAIL (5 modules — 19 handler files total)

#### GL — 11 files

Only `gl_detail.rs` uses `VerifiedClaims`. All other route files take
`tenant_id` from query params:

- `routes/account_activity.rs` — `tenant_id: String` in `AccountActivityQuery`
- `routes/balance_sheet.rs` — `tenant_id: String` in `BalanceSheetQuery`
- `routes/cashflow.rs` — `tenant_id: String` in query struct
- `routes/close_checklist.rs` — `tenant_id: String` in multiple structs
- `routes/fx_rates.rs` — `tenant_id: String` in multiple structs
- `routes/income_statement.rs` — `tenant_id: String` in query struct
- `routes/period_close.rs` — `tenant_id: String` in multiple structs
- `routes/period_summary.rs` — `tenant_id: String` in query struct
- `routes/reporting_currency.rs` — `tenant_id: String` in query struct
- `routes/revrec.rs` — `tenant_id: String` in multiple structs
- `routes/trial_balance.rs` — `tenant_id: String` in query struct

#### TTP — 2 files

`metering.rs` correctly uses `VerifiedClaims`. Two files remain:

- `http/billing.rs` — `tenant_id: Uuid` in `BillingRunRequest` (JSON body)
- `http/service_agreements.rs` — `tenant_id: Uuid` in `ListQuery` (query param)

#### Consolidation — 2 files

`config.rs` correctly uses `VerifiedClaims`. Two files remain:

- `http/consolidate.rs` — `tenant_id: String` in `ConsolidateQuery`
- `http/intercompany.rs` — `tenant_id: String` in multiple structs

#### Inventory — 3 files

`http/` handlers were fixed by bd-30mj. `routes/` handlers remain:

- `routes/items.rs` — `tenant_id: String` in `TenantQuery`
- `routes/locations.rs` — `tenant_id: String` in multiple structs
- `routes/uom.rs` — `tenant_id: String` in query struct

#### AR — 1 file

Most handlers use `VerifiedClaims`. One file remains:

- `routes/tax.rs` — `app_id: String` in JSON body, used as tenant_id

---

## Additional Observations

1. **Outbound header (OK):** `ar/integrations/party_client.rs` sets `x-app-id`
   on outgoing HTTP calls — inter-service, not a C1 issue.

2. **Admin endpoints (OK):** All modules have `admin.rs` files using
   `X-Admin-Token` + `HeaderMap`. These are internal operations endpoints.

3. **Missing tenant isolation (separate concern):** Inventory `routes/`
   handlers for adjustments, issues, receipts, reservations, and transfers
   reference no tenant_id at all — possible missing tenant isolation entirely.

---

## Remediation Required

19 handler files across 5 modules still derive tenant from client input.
Recommended new C1 fix beads:

1. **GL module** (11 files) — highest volume, critical financial data
2. **TTP module** (2 files) — billing runs, service agreements
3. **Consolidation module** (2 files) — financial consolidation
4. **Inventory routes/** (3 files) — item/location/UoM management
5. **AR tax.rs** (1 file) — tax quote endpoint
