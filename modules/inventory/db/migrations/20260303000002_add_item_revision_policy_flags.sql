-- Add policy flags to item revisions (bd-3eblr).
--
-- These fields are revisioned/effective-dated with item_revisions so policy
-- meaning is stable over time.

ALTER TABLE item_revisions
    ADD COLUMN traceability_level TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN inspection_required BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN shelf_life_days INT,
    ADD COLUMN shelf_life_enforced BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE item_revisions
    ADD CONSTRAINT item_revisions_traceability_level_check
    CHECK (traceability_level IN ('none', 'lot', 'serial', 'batch'));

ALTER TABLE item_revisions
    ADD CONSTRAINT item_revisions_shelf_life_days_positive
    CHECK (shelf_life_days IS NULL OR shelf_life_days > 0);

ALTER TABLE item_revisions
    ADD CONSTRAINT item_revisions_shelf_life_enforced_requires_days
    CHECK (NOT shelf_life_enforced OR shelf_life_days IS NOT NULL);
