-- Party Module: Core Schema
--
-- Unified party model with typed extensions (company / individual).
-- All tables use the `party_` prefix for clear namespacing.
--
-- party (base record) — polymorphic on party_type
-- party_companies     — extension row for company parties (1:1)
-- party_individuals   — extension row for individual parties (1:1)

-- ============================================================
-- ENUMS
-- ============================================================

CREATE TYPE party_type AS ENUM ('company', 'individual');

CREATE TYPE party_status AS ENUM ('active', 'inactive', 'archived');

-- ============================================================
-- BASE PARTY TABLE
-- ============================================================

CREATE TABLE party_parties (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id          TEXT NOT NULL,
    party_type      party_type NOT NULL,
    status          party_status NOT NULL DEFAULT 'active',
    display_name    TEXT NOT NULL,
    email           TEXT,
    phone           TEXT,
    website         TEXT,
    address_line1   TEXT,
    address_line2   TEXT,
    city            TEXT,
    state           TEXT,
    postal_code     TEXT,
    country         TEXT,
    metadata        JSONB,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_parties_app_id     ON party_parties(app_id);
CREATE INDEX idx_party_parties_type       ON party_parties(party_type);
CREATE INDEX idx_party_parties_status     ON party_parties(status);
CREATE INDEX idx_party_parties_app_type   ON party_parties(app_id, party_type);
CREATE INDEX idx_party_parties_app_status ON party_parties(app_id, status);
CREATE INDEX idx_party_parties_email      ON party_parties(email) WHERE email IS NOT NULL;
CREATE INDEX idx_party_parties_created_at ON party_parties(created_at);

-- ============================================================
-- COMPANY EXTENSION (1:1 with party_parties WHERE party_type = 'company')
-- ============================================================

CREATE TABLE party_companies (
    party_id                UUID PRIMARY KEY REFERENCES party_parties(id) ON DELETE CASCADE,
    legal_name              TEXT NOT NULL,
    trade_name              TEXT,
    registration_number     TEXT,
    tax_id                  TEXT,
    country_of_incorporation TEXT,
    industry_code           TEXT,
    founded_date            DATE,
    employee_count          INTEGER,
    annual_revenue_cents    BIGINT,
    currency                TEXT DEFAULT 'usd',
    metadata                JSONB,
    created_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_companies_legal_name         ON party_companies(legal_name);
CREATE INDEX idx_party_companies_registration_number ON party_companies(registration_number) WHERE registration_number IS NOT NULL;
CREATE INDEX idx_party_companies_tax_id             ON party_companies(tax_id) WHERE tax_id IS NOT NULL;
CREATE INDEX idx_party_companies_industry_code      ON party_companies(industry_code) WHERE industry_code IS NOT NULL;

-- ============================================================
-- INDIVIDUAL EXTENSION (1:1 with party_parties WHERE party_type = 'individual')
-- ============================================================

CREATE TABLE party_individuals (
    party_id        UUID PRIMARY KEY REFERENCES party_parties(id) ON DELETE CASCADE,
    first_name      TEXT NOT NULL,
    last_name       TEXT NOT NULL,
    middle_name     TEXT,
    date_of_birth   DATE,
    tax_id          TEXT,
    nationality     TEXT,
    job_title       TEXT,
    department      TEXT,
    metadata        JSONB,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_individuals_last_name  ON party_individuals(last_name);
CREATE INDEX idx_party_individuals_full_name  ON party_individuals(last_name, first_name);
CREATE INDEX idx_party_individuals_tax_id     ON party_individuals(tax_id) WHERE tax_id IS NOT NULL;
