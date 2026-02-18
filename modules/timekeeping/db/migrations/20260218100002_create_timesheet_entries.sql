-- Timekeeping: Timesheet Entries (append-only with corrections)
--
-- Append-only strategy: entries are never updated or deleted.
-- Corrections insert a new row with the same entry_id and incremented version.
-- Only the latest version (is_current = TRUE) counts toward totals.
-- Voiding inserts a correction with minutes = 0 and entry_type = 'void'.
--
-- This gives full audit trail: every change is a new row.

CREATE TYPE tk_entry_type AS ENUM (
    'original',    -- first submission
    'correction',  -- replaces a previous version
    'void'         -- cancels the entry (minutes = 0)
);

CREATE TABLE tk_timesheet_entries (
    id          BIGSERIAL PRIMARY KEY,
    entry_id    UUID NOT NULL,              -- logical entry identifier (same across versions)
    version     INT NOT NULL DEFAULT 1,     -- monotonically increasing per entry_id
    app_id      VARCHAR(50) NOT NULL,
    employee_id UUID NOT NULL REFERENCES tk_employees(id) ON DELETE RESTRICT,
    project_id  UUID REFERENCES tk_projects(id) ON DELETE RESTRICT,
    task_id     UUID REFERENCES tk_tasks(id) ON DELETE RESTRICT,
    work_date   DATE NOT NULL,
    -- Duration stored as integer minutes (avoids floating-point rounding)
    minutes     INT NOT NULL CHECK (minutes >= 0),
    description TEXT,
    entry_type  tk_entry_type NOT NULL DEFAULT 'original',
    -- Only the latest version is TRUE; older versions flipped to FALSE on correction
    is_current  BOOLEAN NOT NULL DEFAULT TRUE,
    created_by  UUID,                       -- user who created this version
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Append-only versioning: one row per (entry_id, version)
    CONSTRAINT tk_entries_entry_version_unique
        UNIQUE (entry_id, version)
);

-- Primary query: current entries for an employee in a date range
CREATE INDEX tk_entries_app_employee_date
    ON tk_timesheet_entries(app_id, employee_id, work_date)
    WHERE is_current = TRUE;

-- Project-level time aggregation
CREATE INDEX tk_entries_app_project_date
    ON tk_timesheet_entries(app_id, project_id, work_date)
    WHERE is_current = TRUE;

-- Full history for a given logical entry (audit trail)
CREATE INDEX tk_entries_entry_id
    ON tk_timesheet_entries(entry_id);

-- Tenant-scoped queries
CREATE INDEX tk_entries_app_id
    ON tk_timesheet_entries(app_id);

-- Date range scans for reporting / export
CREATE INDEX tk_entries_app_date
    ON tk_timesheet_entries(app_id, work_date)
    WHERE is_current = TRUE;
