-- Add due_notified_at to maintenance_plan_assignments.
-- Used by the scheduler tick for idempotent due-event emission.
-- NULL = not yet notified (scheduler will pick it up when due).
-- Reset to NULL when a work order is completed and next_due is recomputed.

ALTER TABLE maintenance_plan_assignments
    ADD COLUMN due_notified_at TIMESTAMP WITH TIME ZONE;

CREATE INDEX idx_plan_assignments_due_candidates
    ON maintenance_plan_assignments (state, due_notified_at)
    WHERE state = 'active' AND due_notified_at IS NULL;
