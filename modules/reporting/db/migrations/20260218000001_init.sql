-- Reporting module: initial migration
-- Creates the schema skeleton. Full schema (KPI cache, statement cache,
-- ingestion checkpoints) is added by subsequent migrations (bd-3sli).

-- Placeholder table so sqlx migrate has at least one migration to apply.
-- This keeps the service bootable before bd-3sli schema lands.
CREATE TABLE IF NOT EXISTS reporting_schema_version (
    id          SERIAL PRIMARY KEY,
    version     TEXT        NOT NULL,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO reporting_schema_version (version) VALUES ('20260218000001_init');
