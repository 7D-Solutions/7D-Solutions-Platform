-- Timekeeping: Add billing fields to entries + create billing run tables
--
-- Extends timesheet entries with optional billing rate linkage and billable flag.
-- Billing runs aggregate billable entries into AR invoices.

-- 1. Add billing fields to timesheet entries
ALTER TABLE tk_timesheet_entries
    ADD COLUMN IF NOT EXISTS billing_rate_id UUID REFERENCES tk_billing_rates(id),
    ADD COLUMN IF NOT EXISTS billable BOOLEAN NOT NULL DEFAULT FALSE;

-- 2. Billing runs table — one run per app + period + customer
CREATE TABLE tk_billing_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id          VARCHAR(50) NOT NULL,
    ar_customer_id  INT NOT NULL,
    from_date       DATE NOT NULL,
    to_date         DATE NOT NULL,
    amount_cents    BIGINT NOT NULL DEFAULT 0,
    ar_invoice_id   INT,                        -- AR invoice ID created for this run
    idempotency_key TEXT NOT NULL,              -- hash of (app_id, from_date, to_date, ar_customer_id)
    status          TEXT NOT NULL DEFAULT 'completed',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT tk_billing_runs_idempotency_unique UNIQUE (idempotency_key)
);

CREATE INDEX tk_billing_runs_app_period
    ON tk_billing_runs(app_id, from_date, to_date);

-- 3. Links between billing runs and the entries they covered
CREATE TABLE tk_billing_run_entries (
    billing_run_id  UUID NOT NULL REFERENCES tk_billing_runs(id) ON DELETE CASCADE,
    entry_id        UUID NOT NULL,
    amount_cents    BIGINT NOT NULL,

    PRIMARY KEY (billing_run_id, entry_id)
);

CREATE INDEX tk_billing_run_entries_entry_id
    ON tk_billing_run_entries(entry_id);
