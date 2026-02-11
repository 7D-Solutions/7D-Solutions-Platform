ALTER TABLE credentials
ADD COLUMN IF NOT EXISTS failed_login_count INT NOT NULL DEFAULT 0;

ALTER TABLE credentials
ADD COLUMN IF NOT EXISTS last_failed_login_at TIMESTAMPTZ;

ALTER TABLE credentials
ADD COLUMN IF NOT EXISTS lock_until TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_credentials_lock_until ON credentials(lock_until);
