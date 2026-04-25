-- Add ar_customer_id to opportunities and leads for CRM ↔ AR linkage.
-- Populated by the ar.customer_created consumer when party_id matches.

ALTER TABLE opportunities ADD COLUMN ar_customer_id INTEGER;
ALTER TABLE leads ADD COLUMN ar_customer_id INTEGER;
