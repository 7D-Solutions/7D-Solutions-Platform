-- AP bill attachment links.
--
-- Records the association between a vendor bill and a document attachment.
-- Populated by the AP consumer that listens to docmgmt.attachment.created events.
-- Idempotent: UNIQUE constraint on (bill_id, attachment_id) prevents duplicates.

CREATE TABLE IF NOT EXISTS bill_attachments (
    id              BIGSERIAL PRIMARY KEY,
    bill_id         UUID    NOT NULL REFERENCES vendor_bills (bill_id),
    attachment_id   UUID    NOT NULL,
    tenant_id       TEXT    NOT NULL,
    linked_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_bill_attachment UNIQUE (bill_id, attachment_id)
);

CREATE INDEX IF NOT EXISTS idx_bill_attachments_bill
    ON bill_attachments (tenant_id, bill_id);
