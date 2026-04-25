-- Shipping-Receiving: Inbound Carrier Tracking (bd-4w4bg)
--
-- Adds expected + latest tracking fields to shipments for inbound PO visibility.
--
-- expected_carrier_code / expected_tracking_number:
--   Set via POST /api/shipping-receiving/inbound-shipments/{id}/expected-tracking
--   when a supplier confirms shipment. Populated before physical arrival.
--
-- latest_tracking_status / latest_tracking_dttm / latest_tracking_location:
--   Updated by the carrier webhook pipeline when a webhook event arrives for
--   expected_tracking_number. Informational only — never advances inbound_status.
--
-- Invariant (bd-4w4bg, 2026-04-24): carrier webhook = visibility only.
--   State-machine advance requires dock-scan or manual receipt API call by a human.
--   A carrier "delivered" event does NOT flip inbound_status — the package could
--   be at a gate, wrong address, or loading dock. Audit-trail integrity
--   (especially for aerospace/defense) requires signature + scan before receipt.
--
-- All columns nullable:
--   expected_* NULL  → supplier has not yet provided tracking
--   latest_*   NULL  → no webhook/poll events received for this inbound shipment

ALTER TABLE shipments
    ADD COLUMN expected_carrier_code    TEXT,
    ADD COLUMN expected_tracking_number TEXT,
    ADD COLUMN latest_tracking_status   TEXT,
    ADD COLUMN latest_tracking_dttm     TIMESTAMPTZ,
    ADD COLUMN latest_tracking_location TEXT;

-- Fast lookup: "does this carrier tracking number belong to an inbound PO?"
-- Used by the webhook pipeline to route events to the correct shipment.
CREATE INDEX idx_shipments_expected_tracking
    ON shipments (expected_tracking_number)
    WHERE expected_tracking_number IS NOT NULL;
