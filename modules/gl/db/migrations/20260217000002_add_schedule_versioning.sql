-- Schedule Versioning — Phase 24a Wave 1 (bd-t1b)
-- Adds version tracking and lineage to recognition schedules.
-- Schedules are append-only: never rewrite, always create new version.

ALTER TABLE revrec_schedules
    ADD COLUMN IF NOT EXISTS version INT NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS previous_schedule_id UUID REFERENCES revrec_schedules(schedule_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_revrec_schedules_obligation_version
    ON revrec_schedules(obligation_id, version);
