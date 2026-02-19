-- Fixed Assets: Convert fa_run_status ENUM to TEXT
-- bd-71d7: sqlx FromRow cannot decode Postgres ENUM into Rust String without
-- a custom sqlx::Type impl. Converting to TEXT aligns the schema with the
-- DepreciationRun.status: String field and makes all RETURNING queries work.

-- Drop the column default (which references the ENUM type)
ALTER TABLE fa_depreciation_runs ALTER COLUMN status DROP DEFAULT;

-- Convert the column from ENUM to TEXT (preserves existing values)
ALTER TABLE fa_depreciation_runs ALTER COLUMN status TYPE TEXT
    USING status::TEXT;

-- Restore a text default
ALTER TABLE fa_depreciation_runs ALTER COLUMN status SET DEFAULT 'pending';

-- Drop the ENUM type (CASCADE removes the implicit array type fa_run_status[])
DROP TYPE IF EXISTS fa_run_status CASCADE;
