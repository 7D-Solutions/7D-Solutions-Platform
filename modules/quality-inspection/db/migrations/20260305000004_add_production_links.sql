-- Phase C2: In-process + final inspection production identity links
--
-- Adds wo_id and op_instance_id to inspections table for linking
-- in-process inspections to work order operations and final inspections
-- to work orders + produced lots.

ALTER TABLE inspections
    ADD COLUMN wo_id          UUID,
    ADD COLUMN op_instance_id UUID;

CREATE INDEX idx_inspections_wo ON inspections(wo_id);
CREATE INDEX idx_inspections_wo_op ON inspections(wo_id, op_instance_id);
