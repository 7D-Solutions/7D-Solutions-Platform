# Immutable-Post Compliance Audit

**Bead:** bd-1nqrv  
**Date:** 2026-04-23  
**Auditor:** BrightSparrow

## Principle Under Review

Once a transaction is posted, the original record is immutable. Corrections happen via named offset entries only. No un-post, no edit-and-re-process. This separates grown-up ERPs (NetSuite, SAP, QuickBooks) from shop-floor tools that let users quietly rewrite history.

Two failure modes are tracked:

1. **Backward transition** — the state machine allows moving from a terminal "posted" state back to an editable/draft state
2. **Missing offset entity** — no dedicated correction/reversal record type exists; corrections happen via ad-hoc re-entry or GL-only workarounds

---

## Per-Module Status

| Module | Backward Transitions from Posted? | Offset Entity | Compliant | Gap |
|---|:---:|:---:|:---:|---|
| AR | No ✅ | Credit Notes (`ar_credit_notes`) ✅ | **Yes** ✅ | None |
| GL | No ✅ | Accrual Reversals (`gl_accrual_reversals`) + `reverses_entry_id` ✅ | **Yes** ✅ | None |
| Inventory | No ✅ | Ledger-append status transfers ✅ | **Yes** ✅ | None |
| Shipping-Receiving | No ✅ | RMA Receipts (`rma_receipts`) ✅ | **Yes** ✅ | None |
| AP | No ✅ | GL reversals via outbox (no AP-native entity) ⚠️ | **Partial** ⚠️ | No dedicated AP credit memo for paid-bill corrections |
| Production | No ✅ | None ❌ | **No** ❌ | No time-entry adjustment entity |
| Quality Inspection | No ✅ | None ❌ | **No** ❌ | No re-inspection/correction entity |
| Maintenance | No ✅ | None ❌ | **No** ❌ | No post-close adjustment entity |
| Timekeeping | Pre-review only ⚠️ | None ❌ | **No** ❌ | Recall is pre-review (`submitted→draft`), not a posted-state revert; but no amendment entity exists |

---

## Module Detail

### AR — Compliant ✅

- **Posted state:** `paid`, `void`, `uncollectible` (all terminal)
- **Offset entity:** `ar_credit_notes` — full lifecycle (`draft → approved → issued`); `issued` is terminal and append-only
- **Evidence:** `modules/ar/db/migrations/20260303000006_credit_memo_lifecycle.sql`, `modules/ar/src/lifecycle.rs`
- **No gaps**

---

### GL — Compliant ✅

- **Posted state:** Journal entries are immutable once inserted; period-close constraint prevents posting to closed periods
- **Offset entity:** `gl_accrual_reversals` with `original_accrual_id UNIQUE` (prevents double-reversal); each reversal links to a reversing journal entry via `reverses_entry_id`
- **Evidence:** `modules/gl/db/migrations/20260217000005_create_accrual_reversals.sql`, `modules/gl/src/contracts/gl_entry_reverse_request_v1.rs`, `modules/gl/src/invariants.rs`
- **No gaps**

---

### Inventory — Compliant ✅

- **Posted state:** Stock movements are append-only ledger rows; no mutable state
- **Offset entity:** `inv_status_transfers` — append-only table; corrections are new transfers in the opposing direction
- **Evidence:** `modules/inventory/db/migrations/20260218000014_create_status_transfers.sql`
- **No gaps**

---

### Shipping-Receiving — Compliant ✅

- **Posted state:** `closed`, `cancelled` (terminal for both inbound and outbound shipments)
- **Offset entity:** RMA Receipts (`rma_receipts` + `rma_receipt_items`) with disposition state machine; terminal dispositions are `return_to_stock`, `scrap`
- **Evidence:** `modules/shipping-receiving/src/domain/rma/state_machine.rs`, `modules/shipping-receiving/db/migrations/20260303000005_create_rma_receipts.sql`
- **No gaps**

---

### AP — Partial ⚠️

- **Posted state:** `paid`, `voided` (terminal)
- **Void logic:** `open | matched | approved | partially_paid → voided` — correct; append-only with `reverses_event_id` linkage to original creation event
- **Paid bills are not voidable** — the void guard explicitly rejects `paid` status (`void.rs:68`): _"Paid bills cannot be voided (reversal requires a credit memo)"_
- **Gap:** No `ap_credit_memos` table or entity exists. When a paid bill requires correction, the only path is a manual GL journal entry reversal with no AP-level paper trail. This means AP aging, vendor reconciliation, and 1099 reporting cannot self-consistently identify the correction without joining to GL.
- **Offset naming convention:** Vendor Credit / PO Receipt Correction (Correcting Receipt)
- **Remediation priority:** High — financial accuracy and audit trail for the Aerospace/Defense customer

---

### Production — Non-Compliant ❌

- **Posted state:** Work orders — `closed`, `cancelled` (terminal, enforced by state machine in `modules/production/db/migrations/20260417000001_time_entry_status.sql` and service layer). Time entries — `approved`, `rejected` (terminal)
- **Backward transitions:** None from terminal states ✅
- **Gap:** No `time_entry_adjustments` or equivalent entity. When a time entry is `rejected`, the correct action is to re-enter — but the new entry has no formal link to the rejected original. There is no audit chain showing "entry X was rejected; entry Y is the correction." For manufacturing cost tracking this is a traceability hole: rejected labor time disappears from the work order history.
- **Offset naming convention:** Time Entry Correction
- **Remediation priority:** High — cost tracking is core to the Aerospace/Defense manufacturing use case

---

### Quality Inspection — Non-Compliant ❌

- **Posted state:** `accepted`, `rejected`, `released` (all terminal per check constraint in `modules/quality-inspection/db/migrations/20260305000003_add_disposition_state.sql`)
- **Backward transitions:** None from terminal states ✅
- **Gap:** No `inspection_corrections` or `reinspection_orders` entity. A `rejected` inspection requires a new inspection record with no formal link to the original. Traceability break: cannot query "what was the original rejection that triggered this re-inspection?"
- **Offset naming convention:** Re-inspection
- **Remediation priority:** Medium — important for lot traceability and NCR (non-conformance) reporting, but not blocking day-1 operations

---

### Maintenance — Non-Compliant ❌

- **Posted state:** `closed`, `cancelled` (terminal; `allowed_transitions(Closed)` returns `[]`, enforced in `modules/maintenance/src/domain/work_orders/state_machine.rs:41`)
- **Backward transitions:** None from terminal states ✅
- **Gap:** No `maintenance_adjustments` entity for post-close corrections (labor time errors, parts cost corrections discovered after close). Corrections currently require a new work order with no linkage, losing the "this fixed the same asset/problem" relationship.
- **Offset naming convention:** Void Work Order Completion
- **Remediation priority:** Medium — affects asset maintenance history and cost reporting, but not a day-1 blocker

---

### Timekeeping — Non-Compliant ❌

- **Posted state:** `approved` (terminal — no transitions out of approved)
- **Backward transitions:** Recall only works from `submitted → draft` (pre-review), _not_ from `approved` or `rejected`. The `recall()` service function guards `current.status != ApprovalStatus::Submitted` and rejects anything else. This is not a posted-state revert.
- **Gap:** No `timesheet_amendments` entity. A rejected timesheet is recalled, re-edited, and resubmitted — creating a new version of the same approval request without a formal link to the prior rejection. The `tk_approval_actions` audit trail records the `recall` action but does not capture _what changed_ between the two submissions.
- **Additional note:** The recall path (`submitted → draft`) is sound for the pre-review phase. The true compliance gap is the absence of an amendment record that explains why the resubmission differs from the original.
- **Offset naming convention:** Time Entry Correction
- **Remediation priority:** Low — the audit trail is functional; the missing piece is formal change documentation between versions

---

## Remediation Bead Sequence

Ranked by customer impact for the Aerospace/Defense (Fireproof ERP) customer.

| Priority | Module | Bead Title | Rationale |
|---|---|---|---|
| 1 (High) | AP | `feat(ap): AP credit memo entity for paid-bill corrections` | Paid-bill corrections today are GL-only; AP aging and vendor reconciliation are blind to them |
| 2 (High) | Production | `feat(production): time-entry correction entity linking rejected entries` | Manufacturing cost tracking for Aerospace/Defense depends on a complete, traceable labor record |
| 3 (Medium) | Quality Inspection | `feat(quality-inspection): re-inspection record linked to original rejection` | Lot traceability and NCR audit trails require a formal inspection lineage |
| 4 (Medium) | Maintenance | `feat(maintenance): post-close work-order adjustment entity` | Asset cost history and maintenance reporting lose fidelity without closed-WO correction records |
| 5 (Low) | Timekeeping | `feat(timekeeping): timesheet amendment entity for resubmission change capture` | Current recall+resubmit is functional; amendment adds formal diff capture between versions |

All five offset entities should follow the platform's established append-only pattern:
- Immutable once in terminal state
- `reverses_id` or `original_id` FK to the source record
- Idempotency key
- Outbox event on creation
