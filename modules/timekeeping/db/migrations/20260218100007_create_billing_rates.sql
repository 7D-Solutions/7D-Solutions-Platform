-- Timekeeping: Billing Rates
--
-- Billing rates define a named hourly price used when generating AR invoices
-- from billable timesheet entries. Each entry may reference a billing rate.

CREATE TABLE tk_billing_rates (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              VARCHAR(50) NOT NULL,
    name                TEXT NOT NULL,
    rate_cents_per_hour INT NOT NULL CHECK (rate_cents_per_hour > 0),
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT tk_billing_rates_app_name_unique UNIQUE (app_id, name)
);

CREATE INDEX tk_billing_rates_app_id
    ON tk_billing_rates(app_id)
    WHERE is_active = TRUE;
