-- Add 'degraded' to the allowed tenant statuses.
--
-- 'degraded' means the tenant was provisioned but one or more modules
-- failed their readiness poll. The tenant exists but is not fully
-- operational. Introduced with GAP-16 (bd-exgu3).

ALTER TABLE tenants
    DROP CONSTRAINT IF EXISTS tenants_status_check;

ALTER TABLE tenants
    ADD CONSTRAINT tenants_status_check
    CHECK (status IN (
        'pending', 'provisioning', 'active', 'degraded',
        'failed', 'suspended', 'deleted', 'trial', 'past_due'
    ));
