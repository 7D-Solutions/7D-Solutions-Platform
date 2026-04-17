-- Default pipeline stages and activity types are seeded per tenant via API on first use.
-- This migration is intentionally empty — seeding is idempotent and handled at the
-- application layer (domain::stage::seed_default_stages) to avoid cross-tenant
-- conflicts in shared-DB deployments.
SELECT 1;
