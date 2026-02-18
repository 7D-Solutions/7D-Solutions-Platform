-- inv_status_transfers: append-only ledger of status bucket transitions.
--
-- Each row records one transfer of quantity between status buckets.
-- Idempotency is enforced via inv_idempotency_keys (same table as receipts/issues).
-- Depends on: 011 (status_buckets — inv_item_status enum + item_on_hand_by_status)

CREATE TABLE inv_status_transfers (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT    NOT NULL,
    item_id         UUID    NOT NULL REFERENCES items(id),
    warehouse_id    UUID    NOT NULL,
    from_status     inv_item_status NOT NULL,
    to_status       inv_item_status NOT NULL,
    quantity        BIGINT  NOT NULL CHECK (quantity > 0),
    event_id        UUID    NOT NULL,
    transferred_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    CONSTRAINT inv_status_transfers_check_statuses CHECK (from_status <> to_status)
);

CREATE INDEX idx_status_transfers_item
    ON inv_status_transfers (tenant_id, item_id, warehouse_id);
