-- Add item_id column to po_lines so inventory item references round-trip through AP.
--
-- Before this migration, item_id was folded into the description column as
-- "item:{uuid}", making it unrecoverable downstream (receipt/bill matching).
-- This column stores it verbatim; description retains its own value.
--
-- No FK to inventory: cross-module DB reads are forbidden. AP treats item_id
-- as an opaque UUID and echoes it in responses and the ap.po_created event.

ALTER TABLE po_lines ADD COLUMN IF NOT EXISTS item_id UUID;

CREATE INDEX IF NOT EXISTS idx_po_lines_item_id ON po_lines (item_id) WHERE item_id IS NOT NULL;
