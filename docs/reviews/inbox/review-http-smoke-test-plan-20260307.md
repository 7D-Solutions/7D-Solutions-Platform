# HTTP Smoke Test Plan — Full Route Coverage

## Request
Create exhaustive HTTP-level smoke tests for every API route in the 7D Solutions Platform. 443 routes across 24 modules. Current HTTP-level coverage is 6%.

## What we need from you
1. Review this plan for completeness — are we missing any risk areas?
2. For each module, identify the critical happy-path sequences (e.g., create vendor -> create PO -> approve PO -> receive)
3. Flag any routes where testing order matters (state machine dependencies)
4. Identify any cross-module test sequences we should add (e.g., BOM -> Production -> QI -> Shipping)

## Current State
- 148 E2E test files exist but 130 bypass HTTP — they call Rust domain functions directly
- Only 17 test files + 4 stress tests use reqwest (HTTP client)
- Business logic is well-tested; HTTP wiring, auth, serialization are NOT

## Test Pattern (every test must follow this)
1. Use reqwest to hit real HTTP endpoints (no domain-layer shortcuts)
2. Create real data in real databases via API calls (seed tenant, entities)
3. Verify: correct status code, valid JSON response, no SQL/stack traces leaked
4. Verify auth: unauthenticated request -> 401/403
5. Use unique tenant_id = Uuid::new_v4() for isolation
6. All tests run against live Docker containers

## Modules and Route Counts

| Module | Routes | HTTP-Tested | Gap |
|--------|--------|-------------|-----|
| AR | 52 | 7 | 45 |
| Inventory | 46 | 1 | 45 |
| GL | 34 | 3 | 31 |
| Timekeeping | 31 | 0 | 31 |
| AP | 27 | 7 | 20 |
| Production | 26 | 0 | 26 |
| Consolidation | 21 | 0 | 21 |
| Maintenance | 20 | 0 | 20 |
| Treasury | 20 | 1 | 19 |
| BOM | 18 | 0 | 18 |
| Notifications | 18 | 0 | 18 |
| Doc-Mgmt | 17 | 0 | 17 |
| Quality Inspection | 15 | 0 | 15 |
| Shipping-Receiving | 15 | 0 | 15 |
| Fixed Assets | 13 | 0 | 13 |
| Identity-Auth | 12 | 4 | 8 |
| Party | 12 | 7 | 5 |
| Reporting | 11 | 0 | 11 |
| PDF Editor | 10 | 0 | 10 |
| Integrations | 10 | 3 | 7 |
| Payments | 9 | 0 | 9 |
| Workforce-Competence | 7 | 0 | 7 |
| Workflow | 6 | 0 | 6 |
| Subscriptions | 5 | 1 | 4 |
| Control Plane | 4 | 1 | 3 |
| TTP | 4 | 0 | 4 |
| Numbering | 2 | 0 | 2 |
| **TOTAL** | **443** | **30** | **413** |

## Proposed Bead Structure (one per module, split large modules)

### AR (split into 4 beads)
1. AR Customer + Invoice CRUD (customers, invoices, finalize, bill-usage)
2. AR Credit + Disputes (credit-notes, credit-memos, write-off, disputes, refunds)
3. AR Payments + Webhooks (payment-methods, payments/allocate, webhooks, dunning)
4. AR Tax + Aging + Recon + Admin (tax config/reports, aging, recon, events, admin)

### Inventory (split into 4 beads)
1. Inventory Items CRUD (items, deactivate, make-buy, uom-conversions, history)
2. Inventory Transactions (receipts, issues, adjustments, transfers, status-transfers)
3. Inventory Lots + Serials + Reservations (lots, serials, tracing, reservations, cycle-count)
4. Inventory Locations + Policies + Valuation (locations, warehouses, reorder-policies, valuation, labels, expiry)

### GL (split into 3 beads)
1. GL Period Management (periods close/reopen/validate, approvals, checklist, summary)
2. GL RevRec + Accruals (revrec contracts/schedules/amendments/runs, accruals templates/create/reverse)
3. GL Reporting + FX + Exports (trial-balance, balance-sheet, income-statement, cash-flow, detail, exports, accounts activity)

### Production (split into 2 beads)
1. Production Work Orders + Operations (workcenters, work-orders lifecycle, operations, component-issues, fg-receipt)
2. Production Routings + Time + Downtime (routings, time-entries, downtime)

### Maintenance (split into 2 beads)
1. Maintenance Assets + Plans (assets, readings, calibration, downtime, plans)
2. Maintenance Work Orders (work-orders, transitions, labor, parts, assignments, meter-types)

### Timekeeping (split into 2 beads)
1. Timekeeping Core (employees, projects, tasks, entries, corrections, voids, history)
2. Timekeeping Approvals + Billing + Reporting (approvals workflow, allocations, rates, billing-runs, exports, rollups)

### Single-bead modules (1 each)
- AP (20 untested routes)
- Treasury (19 untested routes)
- Consolidation (21 untested routes)
- BOM (18 untested routes)
- Notifications (18 untested routes)
- Doc-Mgmt (17 untested routes)
- Quality Inspection (15 untested routes)
- Shipping-Receiving (15 untested routes)
- Fixed Assets (13 untested routes)
- Identity-Auth (8 untested routes)
- Reporting (11 untested routes)
- PDF Editor (10 untested routes)
- Payments (9 untested routes)
- Integrations (7 untested routes)
- Workforce-Competence (7 untested routes)
- Workflow (6 untested routes)
- Party (5 untested routes)
- Subscriptions (4 untested routes)
- Control Plane (3 untested routes)
- TTP (4 untested routes)
- Numbering (2 untested routes)

## Total: ~38 beads

## Critical Manufacturing Chain (test in this order)
1. Inventory Items + BOM -> 2. Production Work Orders -> 3. Quality Inspection -> 4. Shipping-Receiving

## Critical Financial Chain
1. Party + AR Customers -> 2. AR Invoices -> 3. Payments -> 4. GL Journals -> 5. Reporting

## Response requested
Drop your review into `docs/reviews/outbox/reviewed-http-smoke-test-plan-20260307.md`
