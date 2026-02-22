-- P51-020: Forecast cache tables for cash flow forecasting
-- bd-1e72: rpt_payment_history + rpt_open_invoices_cache
--
-- Design:
--   rpt_payment_history: historical paid invoices with computed days_to_pay
--   rpt_open_invoices_cache: per-invoice lifecycle tracking (open → paid)
--   Both tables are replay-safe via ON CONFLICT upserts on (tenant_id, invoice_id)

-- ============================================================
-- PAYMENT HISTORY
-- ============================================================
-- Historical paid invoices used to compute the empirical CDF
-- for days-to-pay. Grain: (tenant_id, invoice_id).

CREATE TABLE rpt_payment_history (
    id           BIGSERIAL    PRIMARY KEY,
    tenant_id    TEXT         NOT NULL,
    customer_id  TEXT         NOT NULL,
    invoice_id   TEXT         NOT NULL,
    currency     TEXT         NOT NULL,
    amount_cents BIGINT       NOT NULL,
    issued_at    TIMESTAMPTZ  NOT NULL,
    paid_at      TIMESTAMPTZ  NOT NULL,
    days_to_pay  INT          NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_payment_history_invoice_uq
        UNIQUE (tenant_id, invoice_id)
);

CREATE INDEX idx_rpt_payment_history_forecast
    ON rpt_payment_history (tenant_id, customer_id, currency, paid_at);

COMMENT ON TABLE rpt_payment_history IS
    'Paid invoices with computed days_to_pay for forecast CDF. Grain: (tenant_id, invoice_id).';

-- ============================================================
-- OPEN INVOICES CACHE
-- ============================================================
-- Tracks all invoices from opened → paid. The forecast domain reads
-- status='open' rows to compute at_risk projections.

CREATE TABLE rpt_open_invoices_cache (
    id           BIGSERIAL    PRIMARY KEY,
    tenant_id    TEXT         NOT NULL,
    invoice_id   TEXT         NOT NULL,
    customer_id  TEXT         NOT NULL,
    currency     TEXT         NOT NULL,
    amount_cents BIGINT       NOT NULL,
    issued_at    TIMESTAMPTZ  NOT NULL,
    due_at       TIMESTAMPTZ,
    status       TEXT         NOT NULL DEFAULT 'open',
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_open_invoices_invoice_uq
        UNIQUE (tenant_id, invoice_id)
);

CREATE INDEX idx_rpt_open_invoices_forecast
    ON rpt_open_invoices_cache (tenant_id, customer_id, currency, status, issued_at);

COMMENT ON TABLE rpt_open_invoices_cache IS
    'Invoice lifecycle cache (open/paid) for forecast at_risk projections. Grain: (tenant_id, invoice_id).';
