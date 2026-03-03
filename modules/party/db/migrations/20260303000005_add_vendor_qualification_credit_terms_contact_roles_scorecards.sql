-- Party Module: Vendor Qualification, Credit Terms, Contact Roles, Scorecards
--
-- Aerospace supplier management extensions for the party module.
--
-- party_vendor_qualifications: Tracks vendor qualification status, expiry,
--   and certification references. Qualifications are time-bounded compliance gates.
--
-- party_credit_terms: Payment terms, credit limits, and effective date ranges
--   per party. Supports history via multiple rows with non-overlapping dates.
--
-- party_contact_roles: Structured role assignments per contact per party,
--   with primary flag and effective date tracking.
--
-- party_scorecards: Vendor performance metrics, scores, and review dates.

-- ============================================================
-- VENDOR QUALIFICATIONS
-- ============================================================

CREATE TABLE party_vendor_qualifications (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id            UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id              TEXT NOT NULL,
    qualification_status TEXT NOT NULL DEFAULT 'pending',
    certification_ref   TEXT,
    issued_at           TIMESTAMP WITH TIME ZONE,
    expires_at          TIMESTAMP WITH TIME ZONE,
    notes               TEXT,
    idempotency_key     TEXT,
    metadata            JSONB,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT party_vendor_qual_app_idem_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_party_vendor_qual_party_id   ON party_vendor_qualifications(party_id);
CREATE INDEX idx_party_vendor_qual_app_id     ON party_vendor_qualifications(app_id);
CREATE INDEX idx_party_vendor_qual_app_party  ON party_vendor_qualifications(app_id, party_id);
CREATE INDEX idx_party_vendor_qual_status     ON party_vendor_qualifications(qualification_status);
CREATE INDEX idx_party_vendor_qual_expires    ON party_vendor_qualifications(expires_at)
    WHERE expires_at IS NOT NULL;

-- ============================================================
-- CREDIT TERMS
-- ============================================================

CREATE TABLE party_credit_terms (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id            UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id              TEXT NOT NULL,
    payment_terms       TEXT NOT NULL,
    credit_limit_cents  BIGINT,
    currency            TEXT NOT NULL DEFAULT 'USD',
    effective_from      DATE NOT NULL,
    effective_to        DATE,
    notes               TEXT,
    idempotency_key     TEXT,
    metadata            JSONB,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT party_credit_terms_app_idem_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_party_credit_terms_party_id    ON party_credit_terms(party_id);
CREATE INDEX idx_party_credit_terms_app_id      ON party_credit_terms(app_id);
CREATE INDEX idx_party_credit_terms_app_party   ON party_credit_terms(app_id, party_id);
CREATE INDEX idx_party_credit_terms_effective    ON party_credit_terms(effective_from, effective_to);

-- ============================================================
-- CONTACT ROLES
-- ============================================================

CREATE TABLE party_contact_roles (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id            UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    contact_id          UUID NOT NULL REFERENCES party_contacts(id) ON DELETE CASCADE,
    app_id              TEXT NOT NULL,
    role_type           TEXT NOT NULL,
    is_primary          BOOLEAN NOT NULL DEFAULT false,
    effective_from      DATE NOT NULL,
    effective_to        DATE,
    idempotency_key     TEXT,
    metadata            JSONB,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT party_contact_roles_app_idem_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_party_contact_roles_party_id    ON party_contact_roles(party_id);
CREATE INDEX idx_party_contact_roles_contact_id  ON party_contact_roles(contact_id);
CREATE INDEX idx_party_contact_roles_app_id      ON party_contact_roles(app_id);
CREATE INDEX idx_party_contact_roles_app_party   ON party_contact_roles(app_id, party_id);
CREATE INDEX idx_party_contact_roles_role_type   ON party_contact_roles(role_type);

-- ============================================================
-- SCORECARDS
-- ============================================================

CREATE TABLE party_scorecards (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    party_id            UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id              TEXT NOT NULL,
    metric_name         TEXT NOT NULL,
    score               NUMERIC(5,2) NOT NULL,
    max_score           NUMERIC(5,2) NOT NULL DEFAULT 100.00,
    review_date         DATE NOT NULL,
    reviewer            TEXT,
    notes               TEXT,
    idempotency_key     TEXT,
    metadata            JSONB,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT party_scorecards_app_idem_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_party_scorecards_party_id    ON party_scorecards(party_id);
CREATE INDEX idx_party_scorecards_app_id      ON party_scorecards(app_id);
CREATE INDEX idx_party_scorecards_app_party   ON party_scorecards(app_id, party_id);
CREATE INDEX idx_party_scorecards_metric      ON party_scorecards(metric_name);
CREATE INDEX idx_party_scorecards_review_date ON party_scorecards(review_date);
