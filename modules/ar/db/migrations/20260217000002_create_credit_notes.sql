-- AR Credit Notes (bd-1gt)
--
-- Credit notes are formal financial artifacts that compensate for overbilling
-- or adjustments against a finalized invoice. They are append-only: no UPDATEs
-- or DELETEs on credit_note rows (compensating-entry pattern).
--
-- Schema invariants:
--   1. credit_note_id is the stable business key (UUID, caller-supplied for idempotency)
--   2. invoice_id references ar_invoices.id (required — credit notes attach to invoices)
--   3. amount_minor is always positive (credit reduces balance)
--   4. currency must match the originating invoice
--   5. status: 'issued' only for now (append-only — no void/cancellation at DB level)

CREATE TABLE ar_credit_notes (
    id                  SERIAL          PRIMARY KEY,
    -- Stable business key (idempotency anchor, deterministic from business input)
    credit_note_id      UUID            NOT NULL UNIQUE,
    -- Tenant scoping
    app_id              VARCHAR(50)     NOT NULL,
    -- Customer this credit belongs to
    customer_id         VARCHAR(255)    NOT NULL,
    -- Invoice the credit note compensates
    invoice_id          INTEGER         NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    -- Credit amount in minor currency units (e.g. cents). Always positive.
    amount_minor        BIGINT          NOT NULL CHECK (amount_minor > 0),
    -- ISO 4217 currency code (lowercase, e.g. 'usd')
    currency            VARCHAR(3)      NOT NULL,
    -- Human-readable reason (e.g. 'service_credit', 'billing_error', 'dispute_settled')
    reason              TEXT            NOT NULL,
    -- Optional reference to a line item or usage record
    reference_id        VARCHAR(255),
    -- Lifecycle: 'issued' (append-only — status never changes post-insert)
    status              VARCHAR(20)     NOT NULL DEFAULT 'issued',
    -- When the credit note was formally issued
    issued_at           TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    -- Who authorized the credit (service, agent, or user)
    issued_by           VARCHAR(255),
    -- Outbox event ID for correlation (links to events_outbox.event_id)
    outbox_event_id     UUID,
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- Lookup by tenant + invoice (most common query)
CREATE INDEX ar_credit_notes_app_invoice ON ar_credit_notes(app_id, invoice_id);
-- Lookup by customer for balance calculations
CREATE INDEX ar_credit_notes_app_customer ON ar_credit_notes(app_id, customer_id);
-- Temporal ordering for audit
CREATE INDEX ar_credit_notes_issued_at ON ar_credit_notes(issued_at);
