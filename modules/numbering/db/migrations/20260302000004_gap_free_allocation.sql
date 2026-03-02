-- Gap-free allocation mode: reservation/confirm semantics.
--
-- Adds a per-sequence gap_free flag and per-issuance status tracking.
-- When gap_free = TRUE, numbers are allocated as 'reserved' and must be
-- confirmed via a separate call.  Expired reservations are recycled.

ALTER TABLE sequences ADD COLUMN gap_free BOOLEAN NOT NULL DEFAULT FALSE;

-- reservation_ttl_secs: how long a reservation is held before it becomes
-- eligible for recycling.  Default 300 s (5 minutes).
ALTER TABLE sequences ADD COLUMN reservation_ttl_secs INT NOT NULL DEFAULT 300;

-- issued_numbers status: 'confirmed' (default for non-gap-free, backward
-- compat) or 'reserved' (gap-free pending confirmation).
ALTER TABLE issued_numbers ADD COLUMN status VARCHAR(20) NOT NULL DEFAULT 'confirmed';
ALTER TABLE issued_numbers ADD COLUMN expires_at TIMESTAMPTZ;

-- Fast lookup for recyclable reservations (expired and still reserved).
CREATE INDEX idx_issued_recyclable
    ON issued_numbers (tenant_id, entity, number_value)
    WHERE status = 'reserved';
