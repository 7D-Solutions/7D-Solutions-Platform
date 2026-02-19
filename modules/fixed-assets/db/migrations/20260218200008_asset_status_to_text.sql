-- Fixed Assets: Convert fa_asset_status ENUM to TEXT
-- bd-vo7u.1: sqlx FromRow cannot decode Postgres ENUM to Rust String unless
-- explicitly annotated. Converting to TEXT aligns the schema with the Rust
-- domain models and makes all existing queries work without any code changes.

-- Drop the column default (which references the ENUM type)
ALTER TABLE fa_assets ALTER COLUMN status DROP DEFAULT;

-- Convert the column from ENUM to TEXT (preserves existing values as strings)
ALTER TABLE fa_assets ALTER COLUMN status TYPE TEXT
    USING status::TEXT;

-- Restore a text default
ALTER TABLE fa_assets ALTER COLUMN status SET DEFAULT 'draft';

-- Drop the ENUM type (CASCADE removes the implicit array type fa_asset_status[])
DROP TYPE IF EXISTS fa_asset_status CASCADE;
