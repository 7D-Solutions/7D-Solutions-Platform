-- Payment Terms Schedule
--
-- Structured payment terms that can be assigned to vendor invoices.
-- Supports net terms (e.g. Net 30), discount terms (e.g. 2/10 Net 30),
-- and installment schedules (JSONB).
--
-- Tenant-scoped: term_code is unique per tenant.
-- Idempotent: optional idempotency_key prevents duplicate creation.

CREATE TABLE payment_terms (
    term_id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id            TEXT NOT NULL,
    term_code            TEXT NOT NULL,
    description          TEXT NOT NULL DEFAULT '',
    days_due             INT NOT NULL CHECK (days_due >= 0),
    discount_pct         DOUBLE PRECISION NOT NULL DEFAULT 0,
    discount_days        INT NOT NULL DEFAULT 0 CHECK (discount_days >= 0),
    installment_schedule JSONB,
    idempotency_key      TEXT,
    is_active            BOOLEAN NOT NULL DEFAULT TRUE,
    created_at           TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_payment_terms_tenant_code
        UNIQUE (tenant_id, term_code)
);

-- Partial unique index: only enforce idempotency_key uniqueness when non-NULL
CREATE UNIQUE INDEX uq_payment_terms_idempotency
    ON payment_terms (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX idx_payment_terms_tenant ON payment_terms (tenant_id);

-- Add payment terms reference to vendor_bills for assignment
ALTER TABLE vendor_bills
    ADD COLUMN payment_terms_id UUID REFERENCES payment_terms(term_id);

ALTER TABLE vendor_bills
    ADD COLUMN discount_date TIMESTAMP WITH TIME ZONE;

ALTER TABLE vendor_bills
    ADD COLUMN discount_amount_minor BIGINT;
