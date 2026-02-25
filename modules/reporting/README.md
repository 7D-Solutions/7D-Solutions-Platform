# Reporting Module

Read-optimized reporting service. Ingests domain events from GL, AR, AP, Payments, and Inventory to build materialized caches for financial statements, aging reports, KPIs, and cash-flow forecasts.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL (port 5443)
- **Port**: 8096 (default)

## Key Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/reporting/pl` | Profit & Loss statement |
| GET | `/api/reporting/balance-sheet` | Balance sheet |
| GET | `/api/reporting/cashflow` | Cash-flow statement |
| GET | `/api/reporting/ar-aging` | AR aging report |
| GET | `/api/reporting/ap-aging` | AP aging report |
| GET | `/api/reporting/kpis` | Key performance indicators |
| GET | `/api/reporting/forecast` | Cash-flow forecast |
| POST | `/api/reporting/rebuild` | Admin: rebuild caches |

## Database Tables

- `rpt_trial_balance_cache` — GL trial balance snapshots
- `rpt_statement_cache` — pre-built financial statements
- `rpt_ar_aging_cache` — AR aging buckets
- `rpt_ap_aging_cache` — AP aging buckets
- `rpt_cashflow_cache` — cash-flow data
- `rpt_kpi_cache` — KPI snapshots
- `rpt_payment_history` — payment history for forecasting
- `rpt_open_invoices_cache` — open invoices for forecasting
- `rpt_ingestion_checkpoints` — consumer offset tracking

## Events Consumed

| Subject | Source | Description |
|---------|--------|-------------|
| `gl.events.posting.requested` | GL | Journal entry postings |
| `ar.events.ar.invoice_opened` | AR | New invoices |
| `ar.events.ar.invoice_paid` | AR | Invoice payments |
| `ap.events.ap.vendor_bill_created` | AP | New vendor bills |
| `ap.events.ap.vendor_bill_voided` | AP | Voided bills |
| `ap.events.ap.payment_executed` | AP | AP payments |
| `payments.events.payment.succeeded` | Payments | Successful payments |
| `inventory.events.inventory.valuation_snapshot` | Inventory | Inventory valuations |

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `HOST` | No | `0.0.0.0` | Bind address |
| `PORT` | No | `8096` | HTTP port |
| `ENV` | No | `development` | Environment name |
| `CORS_ORIGINS` | No | — | Comma-separated allowed origins |
| `ADMIN_TOKEN` | No | — | Token for admin rebuild endpoint |

## Documentation

- **[REPORTING-MODULE-SPEC.md](./docs/REPORTING-MODULE-SPEC.md)**: Full specification

## Development

```bash
./scripts/cargo-slot.sh build -p reporting
./scripts/cargo-slot.sh test -p reporting
```
