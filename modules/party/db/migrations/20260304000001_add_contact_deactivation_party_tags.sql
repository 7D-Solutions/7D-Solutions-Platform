-- Party Module: Contact soft-delete + Party tags
--
-- Adds deactivated_at to party_contacts for soft-delete (replay-safe).
-- Adds tags array to party_parties for role classification.

-- ============================================================
-- CONTACT SOFT-DELETE
-- ============================================================

ALTER TABLE party_contacts
    ADD COLUMN deactivated_at TIMESTAMP WITH TIME ZONE;

CREATE INDEX idx_party_contacts_active
    ON party_contacts(party_id, app_id)
    WHERE deactivated_at IS NULL;

-- ============================================================
-- PARTY TAGS
-- ============================================================

ALTER TABLE party_parties
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}';

CREATE INDEX idx_party_parties_tags
    ON party_parties USING GIN (tags);
