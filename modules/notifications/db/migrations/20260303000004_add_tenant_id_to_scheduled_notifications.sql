-- Phase 58 Gate A: Add first-class tenant_id column for proper tenant isolation.
--
-- Currently tenant identity is embedded in recipient_ref as "tenant_id:user_ref".
-- A dedicated column enables indexed tenant-scoped queries on DLQ and delivery
-- endpoints, and enforces tenant boundary checks at the SQL level.
--
-- ROLLBACK: ALTER TABLE scheduled_notifications DROP COLUMN IF EXISTS tenant_id;

-- Step 1: Add nullable column (safe — no lock contention on large tables)
ALTER TABLE scheduled_notifications
    ADD COLUMN IF NOT EXISTS tenant_id TEXT;

-- Step 2: Backfill from recipient_ref convention ("tenant_id:user_ref")
-- If recipient_ref has no colon, the whole value becomes tenant_id.
UPDATE scheduled_notifications
SET tenant_id = CASE
    WHEN recipient_ref LIKE '%:%'
    THEN split_part(recipient_ref, ':', 1)
    ELSE recipient_ref
END
WHERE tenant_id IS NULL;

-- Step 3: Set NOT NULL after backfill (all rows now populated)
ALTER TABLE scheduled_notifications
    ALTER COLUMN tenant_id SET NOT NULL;

-- Step 4: Index for tenant-scoped DLQ queries
CREATE INDEX IF NOT EXISTS idx_sn_tenant_id
    ON scheduled_notifications (tenant_id);

CREATE INDEX IF NOT EXISTS idx_sn_tenant_dead_lettered
    ON scheduled_notifications (tenant_id, dead_lettered_at)
    WHERE status = 'dead_lettered';
