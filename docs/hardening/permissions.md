# Platform Permission Map

Permission strings are defined in `platform/security/src/permissions.rs` and enforced via
`RequirePermissionsLayer` on all state-changing (POST / PUT / DELETE) routes.

Naming convention: `<module>.<action>` where action is:
- `mutate` — any write operation (POST / PUT / DELETE)
- `post`   — financial journal-posting (GL convention)
- `read`   — query-only (reserved; not enforced by default)

## Module Permissions

| Module | Permission | Routes Protected |
|--------|-----------|-----------------|
| Accounts Receivable | `ar.mutate` | Customers, invoices, payments, payment-intents, auto-collection POST/PUT/DELETE |
| Payments | `payments.mutate` | (event-driven; reserved for future payment-method write endpoints) |
| Subscriptions | `subscriptions.mutate` | `POST /api/bill-runs/execute` |
| General Ledger | `gl.post` | Period close/validate/reopen, checklist, approvals, FX rates POST, revrec, accruals |
| Notifications | `notifications.mutate` | (event-driven; reserved for future notification-preference write endpoints) |
| Inventory | `inventory.mutate` | Items, receipts, issues, reservations, UoM, adjustments, transfers, cycle-counts, reorder-policies, valuation-snapshots, locations POST/PUT/DELETE |
| Reporting | `reporting.mutate` | `POST /api/reporting/rebuild` |
| Treasury | `treasury.mutate` | Bank/credit-card accounts, reconciliation auto/manual-match, GL link, statement import POST/PUT |
| Accounts Payable | `ap.mutate` | Vendors, purchase orders, bills, allocations, payment-runs POST/PUT |
| Consolidation | `consolidation.mutate` | Groups, entities, COA-mappings, elimination-rules, FX-policies, intercompany POST/PUT/DELETE |
| Timekeeping | `timekeeping.mutate` | Employees, projects, tasks, entries, approvals, allocations, exports POST/PUT/DELETE |
| Fixed Assets | `fixed_assets.mutate` | Categories, assets, depreciation schedule/runs, disposals POST/PUT/DELETE |

## Architecture

JWT claims are extracted by `optional_claims_mw` (graceful — passes if no token present).
`RequirePermissionsLayer` checks the `perms` claim array; returns 401 if no claims, 403 if
the required permission is absent.

Read routes are **not** permission-gated by default — they rely on `AuthzLayer` for tenant
isolation at the infrastructure level.

## Token Format

```json
{
  "sub": "user-uuid",
  "tenant_id": "tenant-uuid",
  "perms": ["ar.mutate", "gl.post"],
  "exp": 1700000000
}
```

Set `JWT_PUBLIC_KEY` (RSA PEM) to enable JWT verification. Without it, `optional_claims_mw`
is a no-op and all mutation endpoints will return 401 (no claims available).
