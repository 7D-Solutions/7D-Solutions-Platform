# Fixed Assets Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Fixed Assets tracks **capital assets** throughout their lifecycle — acquisition, depreciation, and disposal. It automates periodic depreciation calculations, capitalizes assets from AP vendor bills, and posts depreciation/disposal entries to GL. Fixed Assets answers "what capital do we own, what is its book value, and what is the depreciation schedule?"

### Non-Goals

Fixed Assets does **NOT**:
- Own vendor bills or purchase orders (owned by AP)
- Own GL journal entries (posts via `gl.posting.requested`)
- Track consumable inventory (owned by Inventory)

---

## 2. Domain Authority

| Domain Entity | Fixed Assets Authority |
|---|---|
| **Asset Categories** | Depreciation method/life defaults per category |
| **Assets** | Individual fixed asset records: cost basis, in-service date, status |
| **Depreciation Schedules** | Per-asset depreciation plans (method, useful life, residual) |
| **Depreciation Runs** | Periodic batch depreciation execution |
| **Disposals** | Asset retirement with gain/loss calculation |
| **AP Capitalizations** | Links from approved AP bill lines to capitalized assets |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `asset_categories` | Category definitions with depreciation defaults |
| `assets` | Asset records with cost, status, category |
| `depreciation_schedules` | Per-asset depreciation parameters |
| `depreciation_runs` | Batch run headers with status |
| `disposals` | Disposal records with gain/loss |
| `ap_capitalizations` | AP bill line → asset capitalization links |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `fa_category.category_created` — new asset category defined
- `fa_asset.asset_created` — new asset registered
- `fa_asset.asset_updated` — asset record modified
- `fa_asset.asset_deactivated` — asset deactivated
- `fa_depreciation_run.depreciation_run_completed` — batch depreciation finished
- `fa_disposal.asset_disposed` — asset retired with gain/loss
- `gl.posting.requested` — depreciation and disposal GL entries

**Consumes:**
- `ap.vendor_bill_approved` — auto-capitalize capex bill lines

---

## 5. Key Invariants

1. Asset cost basis is immutable after capitalization (adjustments via separate entries)
2. Depreciation runs are idempotent per (asset, period)
3. Disposal gain/loss = proceeds - net book value
4. AP capitalization consumer is idempotent on bill line ID
5. Tenant isolation on every table and query

---

## 6. Integration Map

- **AP** → Fixed Assets consumes `ap.vendor_bill_approved` to auto-capitalize capex lines
- **GL** → Fixed Assets emits `gl.posting.requested` for depreciation and disposal entries
- **Reporting** → future: asset register reports, depreciation forecasts

---

## 7. Roadmap

### v0.1.0 (current)
- Asset category management
- Asset CRUD with cost basis and in-service date
- Straight-line and declining balance depreciation
- Periodic depreciation run execution
- Asset disposal with gain/loss
- AP bill capitalization (auto from approved bills)
- GL posting for depreciation and disposal

### v1.0.0 (proven)
- Asset revaluation and impairment
- Component depreciation (sub-assets)
- Asset transfer between locations/departments
- Insurance and warranty tracking
- Barcode/tag integration for physical audits
