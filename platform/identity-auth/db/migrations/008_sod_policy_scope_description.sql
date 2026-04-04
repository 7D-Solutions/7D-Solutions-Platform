-- Add scope and description columns to sod_policies.
ALTER TABLE sod_policies ADD COLUMN scope TEXT;
ALTER TABLE sod_policies ADD COLUMN description TEXT;
