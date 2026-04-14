-- Add tenant-local presentation / report time zone column.
-- Originally introduced as part of bd-pfk8e (tenant-local timezone handling),
-- but mistakenly added by editing the create_tenant_registry migration in-place.
-- This separate migration restores the per-migration immutability sqlx requires.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS locale_tz VARCHAR(64) NOT NULL DEFAULT 'UTC';

COMMENT ON COLUMN tenants.locale_tz IS 'IANA time zone used for tenant-local report and close boundaries';
