# Financial Invariant Reconciliation

**Tool:** `tools/reconciliation/`
**Run:** Nightly (02:00 UTC via cron)
**Verify:** `./tools/reconciliation/run-all.sh`

## Purpose

Financial databases accumulate invariants that must always hold. Bugs, race
conditions, or manual data corrections can silently violate these invariants.
The reconciliation runner checks live data nightly and emits Prometheus metrics
so violations are detected and alerted before customers notice discrepancies.

## Architecture

```
tools/reconciliation/
  src/
    main.rs           CLI entry point, orchestrates module checks
    config.rs         Env-var-based config (one DATABASE_URL per module)
    metrics.rs        Prometheus textfile exposition format
    checks/
      ar.rs           Accounts Receivable invariant checks
      ap.rs           Accounts Payable invariant checks
      gl.rs           General Ledger invariant checks
      inventory.rs    Inventory invariant checks
      bom.rs          Bill of Materials invariant checks
      production.rs   Production (Work Orders) invariant checks
  run-all.sh          Shell wrapper (buildable, cron-safe)
  alerts/
    recon.rules.yml   Prometheus alerting rules
```

## Prometheus Metrics

Metrics are written to `/var/lib/prometheus/node_exporter/recon.prom` (textfile
collector format) and scraped by Prometheus node_exporter.

```
platform_recon_violations_total{module="<m>", invariant="<i>"}  <count>
platform_recon_last_success_timestamp{module="<m>"}              <unix_ts>
```

- **violations_total**: 0 = clean; > 0 = data integrity violation, count = number of offending rows.
- **last_success_timestamp**: Updated only when a module has zero violations. If violations persist across runs, this timestamp becomes stale and triggers the `PlatformReconRunnerStale` alert.

## Alert Rules

Defined in `tools/reconciliation/alerts/recon.rules.yml`.

| Alert | Condition | Severity |
|-------|-----------|----------|
| `PlatformReconViolationDetected` | `platform_recon_violations_total > 0` | critical |
| `PlatformReconRunnerStale` | Last clean run older than 26h | warning |

## Invariants by Module

---

### AR — Accounts Receivable

**Database env var:** `AR_DATABASE_URL`

#### `invoice_line_total`

> `ar_invoices.amount_cents = SUM(ar_invoice_line_items.amount_cents) + COALESCE(SUM(ar_tax_calculations.tax_amount_cents), 0)` for all non-voided invoices.

A stored invoice total that does not match the sum of its line items plus tax
indicates the header was not updated when lines were added/removed, or a partial
write failure left the data inconsistent.

```sql
SELECT i.id, i.app_id,
       i.amount_cents AS stored,
       COALESCE(line_sum.total, 0) + COALESCE(tax_sum.total, 0) AS computed
FROM ar_invoices i
LEFT JOIN (
    SELECT app_id, invoice_id, SUM(amount_cents) AS total
    FROM ar_invoice_line_items
    GROUP BY app_id, invoice_id
) line_sum ON line_sum.invoice_id = i.id AND line_sum.app_id = i.app_id
LEFT JOIN (
    SELECT app_id, invoice_id, SUM(tax_amount_cents) AS total
    FROM ar_tax_calculations
    GROUP BY app_id, invoice_id
) tax_sum ON tax_sum.invoice_id = i.id AND tax_sum.app_id = i.app_id
WHERE i.status NOT IN ('void', 'voided')
  AND i.amount_cents <> COALESCE(line_sum.total, 0) + COALESCE(tax_sum.total, 0);
```

#### `payment_allocation_cap`

> `SUM(ar_payment_allocations.amount_cents per invoice) <= ar_invoices.amount_cents` for all non-voided invoices.

Over-allocation means the customer was credited more than the invoice face value,
which is a financial data integrity error.

```sql
SELECT i.id, i.app_id, i.amount_cents,
       COALESCE(SUM(a.amount_cents), 0) AS total_allocated
FROM ar_invoices i
LEFT JOIN ar_payment_allocations a ON a.invoice_id = i.id AND a.app_id = i.app_id
WHERE i.status NOT IN ('void', 'voided')
GROUP BY i.id, i.app_id, i.amount_cents
HAVING COALESCE(SUM(a.amount_cents), 0) > i.amount_cents;
```

---

### AP — Accounts Payable

**Database env var:** `AP_DATABASE_URL`

#### `bill_line_total`

> `vendor_bills.total_minor = SUM(bill_lines.line_total_minor) + COALESCE(tax_minor, 0)` for all non-voided bills.

```sql
SELECT b.bill_id, b.tenant_id,
       b.total_minor AS stored,
       COALESCE(SUM(l.line_total_minor), 0) + COALESCE(b.tax_minor, 0) AS computed
FROM vendor_bills b
LEFT JOIN bill_lines l ON l.bill_id = b.bill_id
WHERE b.status NOT IN ('voided')
GROUP BY b.bill_id, b.tenant_id, b.total_minor, b.tax_minor
HAVING b.total_minor <> COALESCE(SUM(l.line_total_minor), 0) + COALESCE(b.tax_minor, 0);
```

#### `payment_allocation_cap`

> `SUM(ap_allocations.amount_minor per bill) <= vendor_bills.total_minor` for all non-voided bills.

```sql
SELECT b.bill_id, b.tenant_id, b.total_minor,
       COALESCE(SUM(a.amount_minor), 0) AS total_allocated
FROM vendor_bills b
LEFT JOIN ap_allocations a ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
WHERE b.status NOT IN ('voided')
GROUP BY b.bill_id, b.tenant_id, b.total_minor
HAVING COALESCE(SUM(a.amount_minor), 0) > b.total_minor;
```

---

### GL — General Ledger

**Database env var:** `GL_DATABASE_URL`

#### `journal_entry_balanced`

> For every journal entry: `SUM(debit_minor) = SUM(credit_minor)`.

Double-entry accounting fundamental. An unbalanced entry means the accounting
equation was violated during a posting.

```sql
SELECT je.id, je.tenant_id,
       COALESCE(SUM(jl.debit_minor), 0) AS total_debits,
       COALESCE(SUM(jl.credit_minor), 0) AS total_credits
FROM journal_entries je
LEFT JOIN journal_lines jl ON jl.journal_entry_id = je.id
GROUP BY je.id, je.tenant_id
HAVING COALESCE(SUM(jl.debit_minor), 0) <> COALESCE(SUM(jl.credit_minor), 0);
```

#### `closed_period_hash_present`

> Every closed accounting period (`is_closed = true`) must have `close_hash IS NOT NULL`.

The period close workflow computes a SHA-256 hash over the period's journal entries
and stores it in `accounting_periods.close_hash`. Its absence indicates the close
workflow was bypassed or the hash was cleared post-close.

```sql
SELECT id, tenant_id, period_start, period_end
FROM accounting_periods
WHERE is_closed = true
  AND close_hash IS NULL;
```

---

### Inventory

**Database env var:** `INVENTORY_DATABASE_URL`

#### `on_hand_matches_ledger`

> `item_on_hand.quantity_on_hand = SUM(inventory_ledger.quantity)` per (tenant_id, item_id, warehouse_id).

The `inventory_ledger` is append-only and authoritative. `item_on_hand` is a
materialised projection. Divergence means either the event consumer fell behind
(missed ledger entries) or a direct write bypassed the write path.

Ledger quantities are signed: positive = stock in, negative = stock out.

```sql
SELECT ioh.tenant_id, ioh.item_id, ioh.warehouse_id,
       SUM(ioh.quantity_on_hand) AS projection_total,
       COALESCE(ledger.qty_sum, 0) AS ledger_total
FROM item_on_hand ioh
LEFT JOIN (
    SELECT tenant_id, item_id, warehouse_id, SUM(quantity) AS qty_sum
    FROM inventory_ledger
    GROUP BY tenant_id, item_id, warehouse_id
) ledger ON ledger.tenant_id = ioh.tenant_id
       AND ledger.item_id    = ioh.item_id
       AND ledger.warehouse_id = ioh.warehouse_id
GROUP BY ioh.tenant_id, ioh.item_id, ioh.warehouse_id, ledger.qty_sum
HAVING SUM(ioh.quantity_on_hand) <> COALESCE(ledger.qty_sum, 0);
```

#### `no_negative_on_hand`

> For items with `tracking_mode = 'none'`, `item_on_hand.quantity_on_hand >= 0`.

Physical stock cannot be negative. A negative value means more stock was issued
than received, indicating a ledger corruption or a missing receipt entry.

Lot-tracked and serial-tracked items are excluded because their semantics differ.

```sql
SELECT ioh.tenant_id, ioh.item_id, ioh.warehouse_id, ioh.quantity_on_hand
FROM item_on_hand ioh
JOIN items i ON i.id = ioh.item_id AND i.tenant_id = ioh.tenant_id
WHERE i.tracking_mode = 'none'
  AND ioh.quantity_on_hand < 0;
```

---

### BOM — Bill of Materials

**Database env var:** `BOM_DATABASE_URL`

#### `revision_status_valid`

> Every `bom_revision` row has `status IN ('draft', 'effective', 'superseded')`.

```sql
SELECT id, bom_id, tenant_id, revision_label, status
FROM bom_revisions
WHERE status IS NULL
   OR status NOT IN ('draft', 'effective', 'superseded');
```

#### `effective_bom_no_zero_qty`

> `bom_lines` belonging to an `'effective'` revision must have `quantity > 0`.

A zero-quantity component on a released BOM causes production planning to
calculate zero material requirements — silently producing incorrect work orders.

```sql
SELECT bl.id, bl.revision_id, bl.tenant_id, bl.component_item_id, bl.quantity
FROM bom_lines bl
JOIN bom_revisions br ON br.id = bl.revision_id
WHERE br.status = 'effective'
  AND bl.quantity <= 0;
```

---

### Production — Work Orders

**Database env var:** `PRODUCTION_DATABASE_URL`

#### `completed_wo_output_cap`

> For closed work orders: `completed_quantity <= planned_quantity * 1.1` (10% overrun tolerance).

A 10% overrun tolerance covers scrap and trial pieces in manufacturing.
Exceeding 10% indicates a data entry error in completion reporting or a guard bypass.

```sql
SELECT work_order_id, tenant_id, planned_quantity, completed_quantity,
       CAST(planned_quantity * 1.1 AS INTEGER) AS max_allowed
FROM work_orders
WHERE status = 'closed'
  AND planned_quantity > 0
  AND completed_quantity > CAST(planned_quantity * 1.1 AS INTEGER);
```

#### `closed_wo_has_actual_end`

> Every work order with `status = 'closed'` must have `actual_end IS NOT NULL`.

```sql
SELECT work_order_id, tenant_id, status, actual_end
FROM work_orders
WHERE status = 'closed'
  AND actual_end IS NULL;
```

---

## Known Limitations

### Production Component Issue Cross-Module Check

The bead specification includes:

> Component issues: `sum(issued_qty) >= bom_required_qty` for 'closed' status WOs.

This invariant requires a three-way join across:
- Production database: `work_orders.bom_revision_id`
- BOM database: `bom_lines.quantity` (required per component)
- Inventory database: `inventory_ledger` (quantities issued to work_order reference)

Cross-database joins are not possible in a single connection. This check is
deferred to a future cross-module reconciliation bead that will use a
read-only data warehouse or ETL snapshot approach.

---

## Runner Operations

### Environment Setup

```bash
export AR_DATABASE_URL="postgres://ar_user:ar_pass@localhost:5432/ar_db"
export AP_DATABASE_URL="postgres://ap_user:ap_pass@localhost:5433/ap_db"
export GL_DATABASE_URL="postgres://gl_user:gl_pass@localhost:5434/gl_db"
export INVENTORY_DATABASE_URL="postgres://inventory_user:inventory_pass@localhost:5435/inventory_db"
export BOM_DATABASE_URL="postgres://bom_user:bom_pass@localhost:5436/bom_db"
export PRODUCTION_DATABASE_URL="postgres://production_user:production_pass@localhost:5437/production_db"

# Write metrics to stdout instead of /var/lib/prometheus/node_exporter
export RECON_METRICS_OUTPUT="-"
```

### Manual Run

```bash
./tools/reconciliation/run-all.sh
```

### Dry Run (no DB connections)

```bash
./tools/reconciliation/run-all.sh --dry-run
```

### Run Only Specific Modules

```bash
./tools/reconciliation/run-all.sh --modules gl,inventory
```

### Build Only

```bash
./scripts/cargo-slot.sh build -p reconciliation
```

### Cron Setup

Add to crontab (runs at 02:00 UTC daily):
```
0 2 * * * /path/to/run-all.sh >> /var/log/recon.log 2>&1
```

### Stale Runner Alert

The `PlatformReconRunnerStale` alert fires when `platform_recon_last_success_timestamp`
is older than 26 hours. This covers one missed nightly run (24h) plus a 2h buffer.

If the alert fires:
1. Check cron job status: `crontab -l | grep recon`
2. Check last run logs: `tail -100 /var/log/recon.log`
3. Check if violations are blocking the timestamp update: look for `PlatformReconViolationDetected` alerts
4. Manual run: `./tools/reconciliation/run-all.sh`
