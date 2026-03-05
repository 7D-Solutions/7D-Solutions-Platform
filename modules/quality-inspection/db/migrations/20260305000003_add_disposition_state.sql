-- Phase C1: Disposition state machine on inspections
--
-- Adds disposition column to inspections table.
-- State machine: pending -> held -> (accepted | rejected | released)
-- Hold/release must be enforced before final disposition.

ALTER TABLE inspections
    ADD COLUMN disposition TEXT NOT NULL DEFAULT 'pending'
        CHECK (disposition IN ('pending', 'held', 'accepted', 'rejected', 'released'));

CREATE INDEX idx_inspections_disposition ON inspections(disposition);
