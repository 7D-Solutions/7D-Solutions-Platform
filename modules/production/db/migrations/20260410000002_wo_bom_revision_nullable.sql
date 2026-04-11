-- Allow work orders to be created without a BOM revision.
-- Composite WO create (POST /api/production/work-orders/create) allocates a
-- WO number from the Numbering service and creates the WO in a single call;
-- the BOM revision and routing are optional at creation time.
ALTER TABLE work_orders ALTER COLUMN bom_revision_id DROP NOT NULL;
