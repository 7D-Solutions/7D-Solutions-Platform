-- CRM Pipeline Module — Initial Schema
-- All tables are tenant-scoped via tenant_id (string, app-id key).

-- ============================================================
-- Outbox & Idempotency (platform pattern)
-- ============================================================

CREATE TABLE events_outbox (
    id           SERIAL PRIMARY KEY,
    event_id     UUID         NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id VARCHAR(255) NOT NULL,
    payload      JSONB        NOT NULL,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_crm_outbox_unpublished ON events_outbox (created_at)
    WHERE published_at IS NULL;

CREATE TABLE processed_events (
    id           SERIAL PRIMARY KEY,
    event_id     UUID         NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    processed_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    processor    VARCHAR(100) NOT NULL
);

CREATE INDEX idx_crm_processed_events_event_id ON processed_events (event_id);

-- ============================================================
-- Pipeline Stage Definitions (tenant-configurable)
-- ============================================================

CREATE TABLE pipeline_stages (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT        NOT NULL,
    stage_code            TEXT        NOT NULL,
    display_label         TEXT        NOT NULL,
    description           TEXT,
    order_rank            INTEGER     NOT NULL,
    is_terminal           BOOLEAN     NOT NULL DEFAULT FALSE,
    is_win                BOOLEAN     NOT NULL DEFAULT FALSE,
    probability_default_pct INTEGER,
    active                BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by            TEXT,
    UNIQUE (tenant_id, stage_code)
);

CREATE INDEX idx_pipeline_stages_tenant ON pipeline_stages (tenant_id, active, order_rank);

-- ============================================================
-- Leads
-- ============================================================

CREATE TABLE leads (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT        NOT NULL,
    lead_number             TEXT        NOT NULL,
    source                  TEXT        NOT NULL,
    source_detail           TEXT,
    company_name            TEXT        NOT NULL,
    contact_name            TEXT,
    contact_email           TEXT,
    contact_phone           TEXT,
    contact_title           TEXT,
    party_id                UUID,
    party_contact_id        UUID,
    status                  TEXT        NOT NULL DEFAULT 'new',
    disqualify_reason       TEXT,
    estimated_value_cents   BIGINT,
    currency                TEXT        NOT NULL DEFAULT 'USD',
    converted_opportunity_id UUID,
    converted_at            TIMESTAMPTZ,
    owner_id                TEXT,
    notes                   TEXT,
    created_by              TEXT        NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, lead_number)
);

CREATE INDEX idx_leads_tenant_status ON leads (tenant_id, status);
CREATE INDEX idx_leads_tenant_owner  ON leads (tenant_id, owner_id);

-- ============================================================
-- Opportunities
-- ============================================================

CREATE TABLE opportunities (
    id                        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                 TEXT        NOT NULL,
    opp_number                TEXT        NOT NULL,
    title                     TEXT        NOT NULL,
    party_id                  UUID        NOT NULL,
    primary_party_contact_id  UUID,
    lead_id                   UUID        REFERENCES leads (id),
    stage_code                TEXT        NOT NULL,
    probability_pct           INTEGER     NOT NULL DEFAULT 0 CHECK (probability_pct BETWEEN 0 AND 100),
    estimated_value_cents     BIGINT,
    currency                  TEXT        NOT NULL DEFAULT 'USD',
    expected_close_date       DATE,
    actual_close_date         DATE,
    close_reason              TEXT,
    competitor                TEXT,
    opp_type                  TEXT        NOT NULL DEFAULT 'new_business',
    priority                  TEXT        NOT NULL DEFAULT 'medium',
    description               TEXT,
    requirements              TEXT,
    external_quote_ref        TEXT,
    sales_order_id            UUID,
    owner_id                  TEXT,
    created_by                TEXT        NOT NULL,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, opp_number)
);

CREATE INDEX idx_opportunities_tenant_stage  ON opportunities (tenant_id, stage_code);
CREATE INDEX idx_opportunities_tenant_party  ON opportunities (tenant_id, party_id);
CREATE INDEX idx_opportunities_tenant_owner  ON opportunities (tenant_id, owner_id);
CREATE INDEX idx_opportunities_tenant_lead   ON opportunities (tenant_id, lead_id);

-- ============================================================
-- Opportunity Stage History (append-only)
-- ============================================================

CREATE TABLE opportunity_stage_history (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT        NOT NULL,
    opportunity_id          UUID        NOT NULL REFERENCES opportunities (id),
    from_stage_code         TEXT,
    to_stage_code           TEXT        NOT NULL,
    probability_pct_at_change INTEGER,
    days_in_previous_stage  INTEGER,
    reason                  TEXT,
    notes                   TEXT,
    changed_by              TEXT        NOT NULL,
    changed_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_stage_history_opportunity ON opportunity_stage_history (opportunity_id, changed_at);
CREATE INDEX idx_stage_history_tenant      ON opportunity_stage_history (tenant_id, changed_at);

-- ============================================================
-- Activity Types (tenant-configurable)
-- ============================================================

CREATE TABLE activity_types (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT        NOT NULL,
    activity_type_code  TEXT        NOT NULL,
    display_label       TEXT        NOT NULL,
    icon                TEXT,
    active              BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by          TEXT,
    UNIQUE (tenant_id, activity_type_code)
);

CREATE INDEX idx_activity_types_tenant ON activity_types (tenant_id, active);

-- ============================================================
-- Activities
-- ============================================================

CREATE TABLE activities (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT        NOT NULL,
    activity_type_code  TEXT        NOT NULL,
    subject             TEXT        NOT NULL,
    description         TEXT,
    activity_date       DATE        NOT NULL,
    duration_minutes    INTEGER,
    lead_id             UUID        REFERENCES leads (id),
    opportunity_id      UUID        REFERENCES opportunities (id),
    party_id            UUID,
    party_contact_id    UUID,
    due_date            DATE,
    is_completed        BOOLEAN     NOT NULL DEFAULT FALSE,
    completed_at        TIMESTAMPTZ,
    assigned_to         TEXT,
    created_by          TEXT        NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_activity_has_entity CHECK (
        lead_id IS NOT NULL OR opportunity_id IS NOT NULL
        OR party_id IS NOT NULL OR party_contact_id IS NOT NULL
    )
);

CREATE INDEX idx_activities_tenant_assigned ON activities (tenant_id, assigned_to, is_completed);
CREATE INDEX idx_activities_tenant_lead     ON activities (tenant_id, lead_id);
CREATE INDEX idx_activities_tenant_opp      ON activities (tenant_id, opportunity_id);
CREATE INDEX idx_activities_due             ON activities (tenant_id, due_date) WHERE is_completed = FALSE;

-- ============================================================
-- Contact Role Attributes (CRM-specific overlay on Party contacts)
-- ============================================================

CREATE TABLE contact_role_attributes (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT        NOT NULL,
    party_contact_id    UUID        NOT NULL,
    sales_role          TEXT        NOT NULL DEFAULT 'unknown',
    is_primary_buyer    BOOLEAN     NOT NULL DEFAULT FALSE,
    is_economic_buyer   BOOLEAN     NOT NULL DEFAULT FALSE,
    is_active           BOOLEAN     NOT NULL DEFAULT TRUE,
    notes               TEXT,
    updated_by          TEXT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, party_contact_id)
);

CREATE INDEX idx_contact_role_attr_tenant ON contact_role_attributes (tenant_id, party_contact_id);

-- ============================================================
-- Label Tables (per-tenant display overrides for canonical values)
-- ============================================================

CREATE TABLE lead_status_labels (
    tenant_id     TEXT NOT NULL,
    canonical     TEXT NOT NULL,
    display_label TEXT NOT NULL,
    PRIMARY KEY (tenant_id, canonical)
);

CREATE TABLE lead_source_labels (
    tenant_id     TEXT NOT NULL,
    canonical     TEXT NOT NULL,
    display_label TEXT NOT NULL,
    PRIMARY KEY (tenant_id, canonical)
);

CREATE TABLE opp_type_labels (
    tenant_id     TEXT NOT NULL,
    canonical     TEXT NOT NULL,
    display_label TEXT NOT NULL,
    PRIMARY KEY (tenant_id, canonical)
);

CREATE TABLE opp_priority_labels (
    tenant_id     TEXT NOT NULL,
    canonical     TEXT NOT NULL,
    display_label TEXT NOT NULL,
    PRIMARY KEY (tenant_id, canonical)
);
