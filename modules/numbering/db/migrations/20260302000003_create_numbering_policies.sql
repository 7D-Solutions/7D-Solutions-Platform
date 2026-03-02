-- Numbering policies: per-tenant, per-entity formatting configuration.
--
-- Each row defines how raw sequence numbers are formatted into human-readable
-- document numbers.  Formatting is pure decoration — it never affects the
-- atomic allocation in `sequences`.
--
-- Pattern tokens:
--   {prefix}  — literal prefix value from the `prefix` column
--   {YYYY}    — 4-digit year from reference date
--   {YY}      — 2-digit year
--   {MM}      — 2-digit month
--   {DD}      — 2-digit day
--   {number}  — raw number (zero-padded to `padding` digits)

CREATE TABLE numbering_policies (
    tenant_id  UUID NOT NULL,
    entity     VARCHAR(100) NOT NULL,
    pattern    VARCHAR(255) NOT NULL DEFAULT '{number}',
    prefix     VARCHAR(50) NOT NULL DEFAULT '',
    padding    INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    version    INT NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, entity)
);
