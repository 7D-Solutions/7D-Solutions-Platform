# Phase 12: Reporting Foundation

**Version:** 1.0
**Status:** Locked
**Date:** 2026-02-13

## Overview

Phase 12 adds **reporting primitives** on top of Phase 11's boundary-validated balances and Phase 10's audit-grade journals. This phase delivers structured read APIs for common GL reporting scenarios—without introducing a reporting engine, analytics layer, or UI framework.

### Goals

1. **Account Activity Report**: Transaction-level detail for a single account over a date range
2. **GL Detail Report**: Multi-account transaction detail with filtering and pagination
3. **Trial Balance Enhancements**: Filtering by account type, date ranges, export formats
4. **Period Summary Report**: Pre-aggregated snapshots by accounting period
5. **Performance Indexes**: Ensure all reports execute in < 500ms at normal scale (100K entries/tenant)

### Non-Goals

**Explicitly out of scope:**
- Financial statements (Balance Sheet, P&L, Cash Flow) → Phase 13+
- Analytics, dashboards, or BI tools → Future
- Report builder UI or visual designer → Future
- Custom report engine or query language → Future
- Drill-down/drill-across OLAP capabilities → Future
- Cross-tenant aggregation or consolidation → Future

### Design Principles

1. **Additive-only**: No schema changes that break Phase 10/11 functionality
2. **Tenant isolation**: All queries scoped by `tenant_id`
3. **Period governance**: Respect `accounting_periods.is_closed` for historical data
4. **Boundary-first**: All reports exposed via HTTP GET endpoints (read path)
5. **Performance guardrails**: Forbid full-table scans; use `account_balances` where possible
6. **Pagination-first**: All multi-record responses paginated (limit, offset)

---

## Data Sources

### Primary Tables

| Table | Purpose | Usage Pattern |
|-------|---------|---------------|
| `account_balances` | Pre-aggregated balances per account | **PREFERRED** for balance queries, period summaries |
| `journal_entries` | Transaction headers | Metadata (date, description, source) |
| `journal_lines` | Transaction details | **REQUIRED** for transaction detail (debit/credit amounts) |
| `accounts` | Chart of Accounts | Account metadata (code, name, type) |
| `accounting_periods` | Period boundaries | Period filtering, closed-period enforcement |

### Data Source Rules

**CRITICAL:** Avoid full-table scans of `journal_lines` at normal scale.

- **Trial Balance**: Use `account_balances` (Phase 11) ✅
- **Period Summary**: Use `account_balances` grouped by period ✅
- **Account Activity**: Query `journal_lines` filtered by `account_ref` + date range ⚠️ (index required)
- **GL Detail**: Query `journal_lines` with multi-column filters ⚠️ (indexes required)

**Escalation trigger**: If any query requires scanning >10K rows without an index, STOP and escalate.

---

## API Specifications

### Base URL
```
http://localhost:8090/api/gl
```

All endpoints require `tenant_id` header:
```
X-Tenant-ID: <uuid>
```

### Error Responses
- `400 Bad Request`: Invalid parameters (missing tenant_id, invalid dates)
- `404 Not Found`: Account not found
- `422 Unprocessable Entity`: Invalid date range (e.g., start > end)
- `500 Internal Server Error`: Database error

---

## 1. Account Activity Report

**Endpoint:** `GET /api/gl/reports/account-activity`

**Purpose:** Transaction-level detail for a single account over a date range.

### Request Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `tenant_id` | Header | Yes | Tenant UUID |
| `account_code` | Query | Yes | Account code (e.g., "1000") |
| `start_date` | Query | Yes | ISO 8601 date (e.g., "2025-01-01") |
| `end_date` | Query | Yes | ISO 8601 date (inclusive) |
| `limit` | Query | No | Records per page (default: 100, max: 1000) |
| `offset` | Query | No | Pagination offset (default: 0) |

### Response Schema

```json
{
  "account": {
    "code": "1000",
    "name": "Cash",
    "type": "asset",
    "normal_balance": "debit"
  },
  "period": {
    "start_date": "2025-01-01",
    "end_date": "2025-01-31"
  },
  "opening_balance": "1000.00",
  "closing_balance": "1500.00",
  "transactions": [
    {
      "entry_id": "uuid",
      "entry_date": "2025-01-15",
      "description": "Invoice INV-001 payment",
      "debit_amount": "500.00",
      "credit_amount": "0.00",
      "running_balance": "1500.00"
    }
  ],
  "pagination": {
    "limit": 100,
    "offset": 0,
    "total_count": 42
  }
}
```

### Data Source
- **Primary**: `journal_lines` filtered by `account_ref` + `journal_entries.entry_date` (JOIN required)
- **Balance calculation**: Use `account_balances` for opening balance, compute running total from transactions

### Performance Target
- **Response time**: < 200ms for 1000 transactions
- **Index required**: `(tenant_id, account_ref, entry_date)` on `journal_lines`

### Business Rules
1. Account must exist and be active
2. Date range must be valid (`start_date <= end_date`)
3. Opening balance = balance at `start_date - 1 day` (from `account_balances`)
4. Running balance = opening + cumulative sum of (debit - credit) for debit-normal accounts

---

## 2. GL Detail Report

**Endpoint:** `GET /api/gl/reports/gl-detail`

**Purpose:** Multi-account transaction detail with filtering.

### Request Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `tenant_id` | Header | Yes | Tenant UUID |
| `start_date` | Query | Yes | ISO 8601 date |
| `end_date` | Query | Yes | ISO 8601 date (inclusive) |
| `account_codes` | Query | No | Comma-separated account codes (e.g., "1000,2000") |
| `account_types` | Query | No | Comma-separated types (e.g., "asset,liability") |
| `limit` | Query | No | Records per page (default: 100, max: 1000) |
| `offset` | Query | No | Pagination offset (default: 0) |

### Response Schema

```json
{
  "period": {
    "start_date": "2025-01-01",
    "end_date": "2025-01-31"
  },
  "filters": {
    "account_codes": ["1000", "2000"],
    "account_types": ["asset"]
  },
  "entries": [
    {
      "entry_id": "uuid",
      "entry_date": "2025-01-15",
      "description": "Invoice INV-001 payment",
      "lines": [
        {
          "line_id": "uuid",
          "account_code": "1000",
          "account_name": "Cash",
          "debit_amount": "500.00",
          "credit_amount": "0.00"
        },
        {
          "line_id": "uuid",
          "account_code": "1200",
          "account_name": "Accounts Receivable",
          "debit_amount": "0.00",
          "credit_amount": "500.00"
        }
      ]
    }
  ],
  "pagination": {
    "limit": 100,
    "offset": 0,
    "total_count": 150
  }
}
```

### Data Source
- **Primary**: `journal_entries` JOIN `journal_lines` JOIN `accounts`
- **Filters**: Apply `entry_date`, `account_ref`, `account.type` filters in WHERE clause

### Performance Target
- **Response time**: < 300ms for 1000 entries
- **Indexes required**:
  - `(tenant_id, entry_date)` on `journal_entries`
  - `(entry_id, account_ref)` on `journal_lines` (composite for JOIN optimization)

### Business Rules
1. Date range must be valid
2. If `account_codes` specified, all accounts must exist
3. Return complete journal entries (all lines) if any line matches filters
4. Entries ordered by `entry_date DESC, created_at DESC`

---

## 3. Trial Balance Enhancements

**Endpoint:** `GET /api/gl/trial-balance` (existing, enhanced)

**Enhancements:**
1. Filter by account type(s)
2. Filter by date range (as-of date)
3. Export formats (JSON, CSV)
4. Include inactive accounts option

### Request Parameters (new)

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `account_types` | Query | No | Comma-separated types (e.g., "asset,liability") |
| `as_of_date` | Query | No | ISO 8601 date (default: today) |
| `include_inactive` | Query | No | Boolean (default: false) |
| `format` | Query | No | "json" or "csv" (default: "json") |

### Response Schema (enhanced)

```json
{
  "as_of_date": "2025-01-31",
  "filters": {
    "account_types": ["asset", "liability"],
    "include_inactive": false
  },
  "accounts": [
    {
      "account_code": "1000",
      "account_name": "Cash",
      "account_type": "asset",
      "normal_balance": "debit",
      "debit_balance": "1500.00",
      "credit_balance": "0.00",
      "is_active": true
    }
  ],
  "totals": {
    "total_debits": "5000.00",
    "total_credits": "5000.00",
    "variance": "0.00"
  },
  "metadata": {
    "generated_at": "2025-02-13T10:00:00Z",
    "account_count": 15
  }
}
```

### Data Source
- **Primary**: `account_balances` (Phase 11)
- **JOIN**: `accounts` for metadata

### Performance Target
- **Response time**: < 100ms (already validated in Phase 11)

---

## 4. Period Summary Report

**Endpoint:** `GET /api/gl/reports/period-summary`

**Purpose:** Pre-aggregated activity by accounting period.

### Request Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `tenant_id` | Header | Yes | Tenant UUID |
| `account_code` | Query | No | Single account code (optional) |
| `account_type` | Query | No | Single account type (optional) |
| `start_period` | Query | Yes | Period ID or date |
| `end_period` | Query | Yes | Period ID or date |

### Response Schema

```json
{
  "filters": {
    "account_code": "1000",
    "start_period": "2025-Q1",
    "end_period": "2025-Q2"
  },
  "periods": [
    {
      "period_id": "uuid",
      "period_name": "2025-Q1",
      "period_start": "2025-01-01",
      "period_end": "2025-03-31",
      "is_closed": true,
      "accounts": [
        {
          "account_code": "1000",
          "account_name": "Cash",
          "opening_balance": "1000.00",
          "total_debits": "5000.00",
          "total_credits": "4500.00",
          "closing_balance": "1500.00"
        }
      ]
    }
  ]
}
```

### Data Source
- **Primary**: `account_balance_snapshots` (new table, Phase 12C)
- **Fallback**: Aggregate from `journal_lines` if snapshots not available (expensive, warn in logs)

### Performance Target
- **Response time**: < 150ms for 4 periods × 20 accounts

### Business Rules
1. Snapshots generated at period-close (background job, future scope)
2. For open periods, compute balances on-the-fly from `account_balances`
3. Closed periods MUST use snapshots (no recalculation)

---

## Database Schema Additions

### New Table: `account_balance_snapshots`

```sql
CREATE TABLE account_balance_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    account_id UUID NOT NULL REFERENCES accounts(id),
    opening_balance DECIMAL(19, 4) NOT NULL,
    total_debits DECIMAL(19, 4) NOT NULL,
    total_credits DECIMAL(19, 4) NOT NULL,
    closing_balance DECIMAL(19, 4) NOT NULL,
    snapshot_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT fk_tenant FOREIGN KEY (tenant_id) REFERENCES tenants(id),
    CONSTRAINT uq_snapshot UNIQUE (tenant_id, period_id, account_id)
);

CREATE INDEX idx_snapshots_period ON account_balance_snapshots(tenant_id, period_id);
CREATE INDEX idx_snapshots_account ON account_balance_snapshots(tenant_id, account_id, period_id);
```

### New Indexes on Existing Tables

```sql
-- Account Activity: Fast lookup by account + date
CREATE INDEX idx_journal_lines_account_date
ON journal_lines(tenant_id, account_ref, entry_date);

-- GL Detail: Entry date range queries
CREATE INDEX idx_journal_entries_date
ON journal_entries(tenant_id, entry_date DESC);

-- GL Detail: Efficient JOIN from entries to lines
CREATE INDEX idx_journal_lines_entry_account
ON journal_lines(entry_id, account_ref);
```

---

## Implementation Tracks

### Track A: Repository + APIs (bd-od3 → bd-2r0/bd-3ln → bd-3js)
1. Create `ReportsRepository` with parameterized queries
2. Implement Account Activity endpoint
3. Implement GL Detail endpoint
4. HTTP E2E tests

### Track B: Trial Balance Enhancements (bd-19t)
1. Add filtering parameters to existing endpoint
2. Implement CSV export
3. Update tests

### Track C: Period Summaries (bd-1qe → bd-1ry → bd-1rj)
1. Create `account_balance_snapshots` table + migration
2. Implement Period Summary endpoint
3. HTTP E2E tests

### Track D: Performance (bd-1hg)
1. Add indexes (after bd-od3 defines queries)
2. Validate < 500ms response times
3. Load test with 100K entries

### Integration: CI Gating (bd-e26)
1. Contract tests for all new endpoints
2. E2E tests from Tracks A + C
3. Performance benchmarks
4. Gate merges on passing tests

---

## Acceptance Criteria

### Functional
- ✅ All 4 report endpoints return correct data structure
- ✅ Pagination works (limit, offset, total_count)
- ✅ Filters apply correctly (account codes, types, dates)
- ✅ Tenant isolation enforced (queries scoped by tenant_id)
- ✅ Period governance respected (closed periods use snapshots)

### Performance
- ✅ Account Activity: < 200ms for 1000 transactions
- ✅ GL Detail: < 300ms for 1000 entries
- ✅ Trial Balance: < 100ms (existing Phase 11 baseline)
- ✅ Period Summary: < 150ms for 4 periods × 20 accounts

### Quality
- ✅ All queries use indexes (no full-table scans in EXPLAIN)
- ✅ HTTP E2E tests validate JSON schema + status codes
- ✅ Contract tests validate DTOs match schema
- ✅ Error handling returns correct HTTP status codes

---

## Escalation Criteria

**STOP and escalate to PearlOwl if:**
1. Any query requires scanning >10K rows without an index
2. Performance targets cannot be met with proposed indexes
3. Requirements drift into financial statements (Balance Sheet, P&L)
4. Cross-tenant aggregation is requested
5. Custom query language or report builder is proposed

---

## Out of Scope (Explicitly Deferred)

- **Financial Statements**: Balance Sheet, Income Statement, Cash Flow Statement → Phase 13+
- **Analytics/BI**: Dashboards, charts, pivot tables → Future
- **Report Builder**: Visual designer, drag-and-drop → Future
- **Export Formats**: Excel, PDF → Future (CSV only in Phase 12)
- **Scheduled Reports**: Email, webhooks → Future
- **Audit Trail UI**: Visual timeline of entries → Future
- **Multi-Currency**: FX conversion, revaluation → Future
- **Consolidation**: Multi-entity, intercompany eliminations → Future

---

## Dependencies

### Upstream (Required)
- ✅ Phase 10: Chart of Accounts, Accounting Periods
- ✅ Phase 11: `account_balances` table, boundary E2E tests

### Downstream (Enabled by Phase 12)
- Phase 13: Financial Statements (uses Period Summary API)
- Phase 14: Analytics Layer (uses GL Detail for drill-down)
- Phase 15: Report Builder (uses all Phase 12 endpoints as primitives)

---

## Migration Path

### From Phase 11 to Phase 12
1. Apply new indexes (Track D)
2. Add `account_balance_snapshots` table (Track C)
3. Deploy new endpoints (Tracks A, B, C)
4. Validate performance benchmarks
5. Update API documentation

**Rollback Plan:**
- New endpoints can be disabled via feature flag (no schema changes break Phase 11)
- Indexes can be dropped if causing write performance issues
- `account_balance_snapshots` table unused until Period Summary endpoint is called

---

## Testing Strategy

### Unit Tests
- Repository queries return correct data structure
- Filters apply correctly
- Pagination calculates offsets correctly

### Integration Tests
- Queries execute in < 500ms with test data (1000 entries)
- Indexes used (verify with EXPLAIN ANALYZE)
- Tenant isolation enforced (cross-tenant queries fail)

### E2E Tests
- HTTP GET requests return 200 OK + valid JSON
- Pagination works (limit, offset, total_count)
- Error cases return correct status codes (400, 404, 422)

### Performance Tests
- Load test with 100K entries per tenant
- Verify < 500ms response times
- Monitor query execution plans (no sequential scans)

---

## Documentation Updates

### Files to Create
- `modules/gl/docs/API-REPORTS.md` - Endpoint documentation with examples
- `modules/gl/docs/PERFORMANCE-BENCHMARKS.md` - Query timing baselines

### Files to Update
- `modules/gl/README.md` - Add Phase 12 section
- `modules/gl/docs/ARCHITECTURE.md` - Document reporting layer

---

## Success Metrics

**Phase 12 is complete when:**
1. All 11 beads closed (bd-2xc through bd-e26)
2. ChatGPT approves deliverables (via PearlOwl)
3. All 4 report endpoints deployed and tested
4. Performance benchmarks met (<500ms)
5. CI gates passing (contract + E2E + performance)

**ChatGPT quote expectation:** "Your GL now has operational reporting that scales."
