-- Party Module: Contacts & Addresses
--
-- party_contacts: links a contact person to a party (typically an org/company).
--   A contact represents a named person with a role at the organization
--   (e.g., "billing contact", "primary contact", "technical contact").
--
-- party_addresses: multiple typed addresses per party.
--   Address types: billing, shipping, registered, mailing, other.
--   One address per type can be marked as primary.

-- ============================================================
-- ADDRESS TYPE ENUM
-- ============================================================

CREATE TYPE party_address_type AS ENUM (
    'billing', 'shipping', 'registered', 'mailing', 'other'
);

-- ============================================================
-- CONTACTS TABLE
-- ============================================================

CREATE TABLE party_contacts (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id    UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id      TEXT NOT NULL,
    first_name  TEXT NOT NULL,
    last_name   TEXT NOT NULL,
    email       TEXT,
    phone       TEXT,
    role        TEXT,            -- e.g. 'billing', 'primary', 'technical'
    is_primary  BOOLEAN NOT NULL DEFAULT false,
    metadata    JSONB,
    created_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_contacts_party_id   ON party_contacts(party_id);
CREATE INDEX idx_party_contacts_app_id     ON party_contacts(app_id);
CREATE INDEX idx_party_contacts_app_party  ON party_contacts(app_id, party_id);
CREATE INDEX idx_party_contacts_email      ON party_contacts(email) WHERE email IS NOT NULL;

-- ============================================================
-- ADDRESSES TABLE
-- ============================================================

CREATE TABLE party_addresses (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id        UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id          TEXT NOT NULL,
    address_type    party_address_type NOT NULL DEFAULT 'other',
    label           TEXT,           -- optional human label like "HQ", "Warehouse"
    line1           TEXT NOT NULL,
    line2           TEXT,
    city            TEXT NOT NULL,
    state           TEXT,
    postal_code     TEXT,
    country         TEXT NOT NULL DEFAULT 'US',
    is_primary      BOOLEAN NOT NULL DEFAULT false,
    metadata        JSONB,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_addresses_party_id   ON party_addresses(party_id);
CREATE INDEX idx_party_addresses_app_id     ON party_addresses(app_id);
CREATE INDEX idx_party_addresses_app_party  ON party_addresses(app_id, party_id);
CREATE INDEX idx_party_addresses_type       ON party_addresses(address_type);
