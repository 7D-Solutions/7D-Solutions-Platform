-- Migration: Add idempotency + linkage columns to ar_metered_usage
--
-- bd-23z: Metered usage ingestion hardening (idempotent capture + outbox)
--
-- Adds:
--   idempotency_key  - caller-supplied UUID; unique constraint enforces no double-count
--   usage_uuid       - stable UUID identifier for event payload (idempotency anchor)
--   unit             - unit label for the quantity (e.g. "calls", "GB")
--   invoice_id       - optional: line item invoice linkage (set by bill-run)
--   line_item_id     - optional: specific invoice line item linkage (set by bill-run)

ALTER TABLE ar_metered_usage
    ADD COLUMN IF NOT EXISTS idempotency_key UUID UNIQUE,
    ADD COLUMN IF NOT EXISTS usage_uuid      UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN IF NOT EXISTS unit            VARCHAR(50) NOT NULL DEFAULT 'units',
    ADD COLUMN IF NOT EXISTS invoice_id      INTEGER REFERENCES ar_invoices(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS line_item_id    INTEGER REFERENCES ar_invoice_line_items(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS ar_metered_usage_idempotency_key ON ar_metered_usage(idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS ar_metered_usage_usage_uuid ON ar_metered_usage(usage_uuid);
