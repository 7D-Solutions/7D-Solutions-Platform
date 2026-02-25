# Reporting Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Reporting is the **cross-module analytics and dashboarding layer**. It consumes events from financial and operational modules, builds cached aggregations, and serves dashboard/report endpoints. Reporting is strictly read-only — it never mutates source module data.

### Non-Goals

Reporting does **NOT**:
- Own any source financial or operational data
- Mutate data in other modules
- Replace module-specific operational views (each module has its own read endpoints)
- Handle ad-hoc SQL queries (future BI integration may provide this)

---

## 2. Domain Authority

| Domain Entity | Reporting Authority |
|---|---|
| **Report Definitions** | Configured report templates and parameters |
| **Reporting Caches** | Pre-computed cross-module aggregations |
| **Forecast Caches** | Forward-looking projection data |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `report_definitions` | Report template configurations |
| `reporting_caches` | Cached aggregate data from multiple modules |
| `forecast_caches` | Forecasting projection data |

---

## 4. Events

**Produces:** None (read-only module)

**Consumes:**
- `gl.posting.requested` — GL transaction data for financial reports
- `payments.payment.succeeded` — payment data for cash flow
- `ap.vendor_bill_created` — AP data for payables dashboards
- `ap.vendor_bill_voided` — AP void corrections
- `ap.payment_executed` — AP payment data
- `inventory.valuation_snapshot_created` — inventory valuation for asset reports
- `ar.invoice_opened` — AR data for receivables dashboards
- `ar.invoice_paid` — AR payment tracking
- `ar.ar_aging_updated` — AR aging for collections dashboards

---

## 5. Key Invariants

1. Reporting never writes to any source module's database
2. Cache staleness is bounded (refresh on event, periodic sweep)
3. All consumed events are idempotent
4. Reports are tenant-scoped — no cross-tenant data leakage
5. Forecast models are clearly labeled as projections, not actuals

---

## 6. Integration Map

- **GL** → journal entry and posting data
- **AR** → invoice, payment, and aging data
- **AP** → bill, payment, and payables data
- **Inventory** → valuation snapshot data
- **Payments** → payment success data
- **All modules** → future: configurable event subscriptions for custom reports

---

## 7. Roadmap

### v0.1.0 (current)
- GL-based financial statement caching
- AR aging and collections dashboards
- AP payables dashboards
- Inventory valuation reporting
- Cash flow reporting from payment events
- Financial forecasting (basic)
- KPI calculation and serving

### v1.0.0 (proven)
- Custom report builder (user-defined aggregations)
- Scheduled report generation and email delivery
- Export to CSV/Excel/PDF
- Comparative period analysis (MoM, YoY)
- BI tool integration (API endpoints for Metabase/Looker)
- Real-time dashboard with WebSocket updates
