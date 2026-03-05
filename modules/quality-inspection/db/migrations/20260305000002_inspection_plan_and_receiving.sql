-- Phase C1: inspection plan characteristics + receiving inspection anchors
--
-- Adds: characteristics JSONB on inspection_plans, sampling fields,
--        receipt_id + part_id + part_revision on inspections for receiving traceability.

-- Inspection plan: characteristics (array of {name, type, nominal, tolerance_low, tolerance_high, uom})
ALTER TABLE inspection_plans
    ADD COLUMN characteristics JSONB NOT NULL DEFAULT '[]'::JSONB,
    ADD COLUMN sampling_method TEXT NOT NULL DEFAULT 'full'
        CHECK (sampling_method IN ('full', 'random', 'aql')),
    ADD COLUMN sample_size    INTEGER;

-- Receiving inspection: link to inventory receipt + part revision anchors
ALTER TABLE inspections
    ADD COLUMN receipt_id    UUID,
    ADD COLUMN part_id       UUID,
    ADD COLUMN part_revision TEXT;

CREATE INDEX idx_inspections_receipt ON inspections(receipt_id);
CREATE INDEX idx_inspections_part_rev ON inspections(part_id, part_revision);
