-- Migration: AR aging projection table
--
-- bd-3cb: AR aging projection v1 (invoices minus payments)
--
-- Stores pre-computed aging buckets per (app_id, customer_id, currency).
-- Updated by the aging projection updater after invoice/payment events.
-- Base case: open balance = invoice amount - successful charges.
--
-- Buckets:
--   current          = not yet due (due_at >= NOW() or due_at IS NULL)
--   days_1_30        = 1-30 days overdue
--   days_31_60       = 31-60 days overdue
--   days_61_90       = 61-90 days overdue
--   days_over_90     = > 90 days overdue
--   total_outstanding = sum of all buckets

CREATE TABLE IF NOT EXISTS ar_aging_buckets (
    id                    SERIAL PRIMARY KEY,
    app_id                VARCHAR(50)  NOT NULL,
    customer_id           INTEGER      NOT NULL REFERENCES ar_customers(id) ON DELETE CASCADE,
    currency              VARCHAR(3)   NOT NULL DEFAULT 'usd',
    -- Balances in minor units (cents)
    current_minor         BIGINT       NOT NULL DEFAULT 0,
    days_1_30_minor       BIGINT       NOT NULL DEFAULT 0,
    days_31_60_minor      BIGINT       NOT NULL DEFAULT 0,
    days_61_90_minor      BIGINT       NOT NULL DEFAULT 0,
    days_over_90_minor    BIGINT       NOT NULL DEFAULT 0,
    total_outstanding_minor BIGINT     NOT NULL DEFAULT 0,
    -- Metadata
    invoice_count         INTEGER      NOT NULL DEFAULT 0,
    calculated_at         TIMESTAMP    NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMP    NOT NULL DEFAULT NOW(),
    CONSTRAINT ar_aging_buckets_unique_customer_currency
        UNIQUE (app_id, customer_id, currency)
);

CREATE INDEX IF NOT EXISTS ar_aging_buckets_app_id      ON ar_aging_buckets(app_id);
CREATE INDEX IF NOT EXISTS ar_aging_buckets_customer_id ON ar_aging_buckets(customer_id);
CREATE INDEX IF NOT EXISTS ar_aging_buckets_updated_at  ON ar_aging_buckets(updated_at);
