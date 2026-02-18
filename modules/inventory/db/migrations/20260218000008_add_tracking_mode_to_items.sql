-- Add tracking_mode to item master
--
-- Determines how stock movements are tracked for this SKU:
--   none   — no lot/serial tracking (default for existing items)
--   lot    — stock moves in lots (lot_code required on receipt/issue)
--   serial — each unit has a unique serial number (serial_codes required)
--
-- Immutable after creation (changing tracking_mode after stock exists
-- would invalidate historical layer associations).

ALTER TABLE items
    ADD COLUMN tracking_mode TEXT NOT NULL DEFAULT 'none';

ALTER TABLE items
    ADD CONSTRAINT items_tracking_mode_check
        CHECK (tracking_mode IN ('none', 'lot', 'serial'));

CREATE INDEX idx_items_tracking_mode ON items(tenant_id, tracking_mode);
