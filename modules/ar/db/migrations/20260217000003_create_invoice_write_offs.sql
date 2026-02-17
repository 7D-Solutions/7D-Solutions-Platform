-- AR Invoice Write-offs (bd-2f2)
--
-- Write-offs are formal financial artifacts that forgive uncollectable debt.
-- They are append-only: no UPDATEs or DELETEs on write-off rows (compensating-entry pattern).
--
-- Schema invariants:
--   1. write_off_id is the stable business key (UUID, caller-supplied for idempotency)
--   2. invoice_id references ar_invoices.id (required — write-offs attach to invoices)
--   3. written_off_amount_minor is always positive (amount of debt forgiven)
--   4. currency must match the originating invoice
--   5. One write-off per invoice (UNIQUE constraint on invoice_id)
--      — write-off adjusts the full open balance; partial write-offs not supported in v1
--   6. status: 'written_off' only (append-only — REVERSAL, not a delete)

CREATE TABLE ar_invoice_write_offs (
    id                          SERIAL          PRIMARY KEY,
    -- Stable business key (idempotency anchor, deterministic from business input)
    write_off_id                UUID            NOT NULL UNIQUE,
    -- Tenant scoping
    app_id                      VARCHAR(50)     NOT NULL,
    -- Invoice the write-off applies to (one write-off per invoice enforced below)
    invoice_id                  INTEGER         NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    -- Customer for denormalized lookups
    customer_id                 VARCHAR(255)    NOT NULL,
    -- Amount written off in minor currency units (e.g. cents). Always positive.
    written_off_amount_minor    BIGINT          NOT NULL CHECK (written_off_amount_minor > 0),
    -- ISO 4217 currency code (lowercase, e.g. 'usd')
    currency                    VARCHAR(3)      NOT NULL,
    -- Human-readable reason (e.g. 'uncollectable', 'bankruptcy', 'dispute_settled')
    reason                      TEXT            NOT NULL,
    -- Lifecycle: 'written_off' (append-only — status never changes post-insert)
    status                      VARCHAR(20)     NOT NULL DEFAULT 'written_off',
    -- When the write-off was formally recorded
    written_off_at              TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    -- Who authorized the write-off (service, agent, or user email)
    authorized_by               VARCHAR(255),
    -- Outbox event ID for correlation (links to events_outbox.event_id)
    outbox_event_id             UUID,
    created_at                  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    -- Enforce exactly one write-off per invoice (v1 constraint)
    CONSTRAINT ar_invoice_write_offs_unique_invoice UNIQUE (invoice_id)
);

-- Lookup by tenant + invoice (most common query)
CREATE INDEX ar_invoice_write_offs_app_invoice ON ar_invoice_write_offs(app_id, invoice_id);
-- Lookup by customer for AR balance calculations
CREATE INDEX ar_invoice_write_offs_app_customer ON ar_invoice_write_offs(app_id, customer_id);
-- Temporal ordering for audit / compliance
CREATE INDEX ar_invoice_write_offs_written_off_at ON ar_invoice_write_offs(written_off_at);
