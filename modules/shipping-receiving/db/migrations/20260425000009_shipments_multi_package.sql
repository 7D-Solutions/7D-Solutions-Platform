-- Shipping-Receiving: Multi-Package Shipments (bd-pv2yi)
--
-- Adds master_tracking_number and package_count to shipments.
-- parent_shipment_id was added in 20260425000008 (bd-4dbh3).
--
-- master_tracking_number: set on the master shipment row for parcel multi-piece
--   shipments (UPS/FedEx). NULL for standard single-package shipments.
--   LTL note: pro_number IS the master; master_tracking_number mirrors
--   tracking_number on the master row and children Vec is empty.
--
-- package_count: total physical packages in the logical shipment.
--   Defaults to 1 for all existing and single-package rows.

ALTER TABLE shipments
    ADD COLUMN master_tracking_number TEXT,
    ADD COLUMN package_count          INTEGER NOT NULL DEFAULT 1;

-- Fast lookup: "give me the master shipment for tracking number X"
CREATE INDEX idx_shipments_master_tracking
    ON shipments (master_tracking_number)
    WHERE master_tracking_number IS NOT NULL;
