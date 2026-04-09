# Bulk Import Design — Inventory Items + BOM Lines

**Bead:** bd-oga9m  
**Status:** Draft  
**Date:** 2026-04-09

---

## Decision Summary

| Question | Decision |
|---|---|
| Module ownership | Inventory owns items import; BOM owns BOM import |
| XLSX library | `calamine` |
| Validation strategy | Collect all row errors; return full error report |
| Error format | Array of `{row, field, message}` objects |
| Batch size limit | 5,000 rows per upload |
| Sync vs async | Synchronous |
| File transport | `multipart/form-data` |

---

## Module Ownership

Both endpoints live in their respective service modules, **not** in `integrations`.

The `integrations` file_job framework is designed for long-running async jobs driven by external systems (Amazon pollers, eBay pollers). Bulk import is a bounded, user-initiated, synchronous operation — adding async job lifecycle overhead gives no benefit and introduces service-to-service coupling. Each module already has direct DB access and the validation logic for its own domain.

- `POST /api/inventory/items/import` → lives in `modules/inventory`
- `POST /api/bom/import` → lives in `modules/bom`

---

## XLSX Library: calamine

`calamine` is the correct choice. It is a pure-Rust, read-only XLSX/ODS/XLS reader with no external system dependencies and minimal transitive crates. Import is a read-only operation; write capability (offered by `umya-spreadsheet`) is unnecessary overhead.

Add to `modules/inventory/Cargo.toml` and `modules/bom/Cargo.toml`:

```toml
calamine = "0.24"
```

---

## Endpoint Shape

### Inventory Items Import

```
POST /api/inventory/items/import
Content-Type: multipart/form-data

Part name: file
Part content: .xlsx binary
```

Response on success (`200 OK`):

```json
{
  "imported": 142,
  "skipped": 0,
  "errors": []
}
```

Response on partial failure (`422 Unprocessable Entity`):

```json
{
  "imported": 0,
  "skipped": 0,
  "errors": [
    { "row": 3, "field": "sku", "message": "SKU 'WIDGET-A' already exists for this tenant" },
    { "row": 7, "field": "tracking_mode", "message": "invalid value 'SERIAL'; expected none|lot|serial" },
    { "row": 12, "field": "cogs_account_ref", "message": "required field is empty" }
  ]
}
```

When any errors are present, **no rows are committed** — the entire upload is atomic. The caller fixes the spreadsheet and re-uploads.

### BOM Import

```
POST /api/bom/import
Content-Type: multipart/form-data

Part name: file
Part content: .xlsx binary
```

Same response shape as items import.

---

## XLSX Column Mapping

### Sheet: `items` (for inventory import)

| Column | Field | Required | Notes |
|---|---|---|---|
| A | `sku` | Yes | Case-preserved, unique per tenant |
| B | `name` | Yes | |
| C | `description` | No | |
| D | `uom` | No | Defaults to `ea` if blank |
| E | `tracking_mode` | Yes | `none`, `lot`, or `serial` |
| F | `make_buy` | No | `make`, `buy`, or blank |
| G | `inventory_account_ref` | Yes | GL account ref, e.g. `1200` |
| H | `cogs_account_ref` | Yes | GL account ref, e.g. `5000` |
| I | `variance_account_ref` | Yes | GL account ref, e.g. `5010` |

Row 1 is the header row and is skipped. Import begins at row 2.

### Sheet: `bom_lines` (for BOM import)

| Column | Field | Required | Notes |
|---|---|---|---|
| A | `parent_sku` | Yes | Must already exist as an inventory item |
| B | `revision_label` | No | Defaults to `A` if blank |
| C | `component_sku` | Yes | Must already exist as an inventory item |
| D | `quantity` | Yes | Positive decimal |
| E | `uom` | No | Overrides component's default UoM |
| F | `scrap_factor` | No | Decimal 0.0–1.0; defaults to 0.0 |
| G | `find_number` | No | Integer; balloon number on engineering drawing |

The import groups rows by `(parent_sku, revision_label)`, creates a `BomHeader` if one does not exist for `parent_sku`, creates a `BomRevision` for `revision_label` if it does not exist, then inserts each `BomLine`. All lookups of `parent_sku` and `component_sku` resolve to item UUIDs before any inserts; missing SKUs are reported as row errors.

---

## Validation Strategy

All rows are validated before any writes. Two passes:

**Pass 1 — Field validation** (pure, no DB):
- Required fields present and non-empty
- `tracking_mode` is one of `none|lot|serial`
- `make_buy` is one of `make|buy` or blank
- `quantity` is a positive finite decimal
- `scrap_factor` is between 0.0 and 1.0 if present

**Pass 2 — DB validation** (bulk lookups, one query per check):
- For items: no SKU in the uploaded batch already exists for the tenant
- For BOM: all `parent_sku` and `component_sku` values resolve to known inventory items for the tenant

Errors from both passes are merged into a single error list, keyed by row number. If the error list is non-empty, return 422 with the full list — no rows are written.

**Batch size guard:** If the spreadsheet contains more than 5,000 data rows (after skipping the header), reject with `413 Payload Too Large` before parsing begins.

---

## Implementation Sketch

```
POST /api/inventory/items/import
  → axum multipart extractor reads the xlsx bytes into memory
  → calamine open_workbook_from_rs reads the workbook
  → iterate Sheet "items" rows 2..N → collect Vec<RawItemRow>
  → if len > 5000 → return 413
  → pass 1: validate each RawItemRow fields → accumulate errors
  → pass 2: bulk query SKUs for tenant → accumulate errors
  → if errors.is_empty() → begin transaction → bulk insert items → commit
  → return 200 { imported: N, skipped: 0, errors: [] }
  → else return 422 { imported: 0, skipped: 0, errors: [...] }
```

The handlers live at:
- `modules/inventory/src/http/import.rs`
- `modules/bom/src/http/import.rs`

No new crates beyond `calamine`. No new module. The `multipart` extractor is already available through `axum`.

---

## What This Design Does Not Cover

The following are out of scope for this design and for bead bd-oga9m:

- **Update/upsert mode**: this design is insert-only; updating existing items via import is a separate feature.
- **Template download endpoint**: serving a pre-formatted xlsx template for users to fill in.
- **Async processing / progress polling**: not needed at the row counts manufacturing imports produce.
- **Import history / audit log**: would require a new table and bead.
- **BOM revision status transitions**: imported revisions land in `draft` status; promoting to `released` uses the existing ECO workflow.
