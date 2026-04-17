-- Customer Complaints module: initial schema
-- Tables: complaints, complaint_activity_log, complaint_resolutions,
--         complaint_status_labels, complaint_severity_labels, complaint_source_labels,
--         complaint_category_codes, cc_outbox, cc_processed_events.

-- ── Complaints ────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS complaints (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT NOT NULL,
    complaint_number        TEXT NOT NULL,
    status                  TEXT NOT NULL DEFAULT 'intake'
                                CHECK (status IN ('intake','triaged','investigating','responded','closed','cancelled')),
    party_id                UUID NOT NULL,
    customer_contact_id     UUID,
    source                  TEXT NOT NULL
                                CHECK (source IN ('phone','email','portal','survey','service_ticket','walk_in','letter','other')),
    source_ref              TEXT,
    severity                TEXT
                                CHECK (severity IS NULL OR severity IN ('low','medium','high','critical')),
    category_code           TEXT,
    title                   TEXT NOT NULL,
    description             TEXT,
    source_entity_type      TEXT,
    source_entity_id        UUID,
    assigned_to             TEXT,
    assigned_at             TIMESTAMPTZ,
    due_date                TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '30 days'),
    overdue_emitted_at      TIMESTAMPTZ,
    received_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at         TIMESTAMPTZ,
    responded_at            TIMESTAMPTZ,
    closed_at               TIMESTAMPTZ,
    outcome                 TEXT
                                CHECK (outcome IS NULL OR outcome IN ('resolved','unresolvable','customer_withdrew','duplicate')),
    created_by              TEXT NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, complaint_number)
);

CREATE INDEX IF NOT EXISTS idx_complaints_tenant_status
    ON complaints (tenant_id, status);

CREATE INDEX IF NOT EXISTS idx_complaints_tenant_party
    ON complaints (tenant_id, party_id);

CREATE INDEX IF NOT EXISTS idx_complaints_tenant_assigned
    ON complaints (tenant_id, assigned_to);

CREATE INDEX IF NOT EXISTS idx_complaints_due_date
    ON complaints (tenant_id, due_date)
    WHERE status NOT IN ('responded', 'closed', 'cancelled');

-- ── Complaint Activity Log ────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS complaint_activity_log (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    complaint_id        UUID NOT NULL REFERENCES complaints(id),
    activity_type       TEXT NOT NULL
                            CHECK (activity_type IN ('status_change','note','customer_communication','internal_communication','attachment_added','assignment_change')),
    from_value          TEXT,
    to_value            TEXT,
    content             TEXT,
    visible_to_customer BOOLEAN NOT NULL DEFAULT FALSE,
    recorded_by         TEXT NOT NULL,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_cc_activity_log_complaint
    ON complaint_activity_log (complaint_id);

CREATE INDEX IF NOT EXISTS idx_cc_activity_log_type
    ON complaint_activity_log (complaint_id, activity_type);

-- ── Complaint Resolutions ─────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS complaint_resolutions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    complaint_id        UUID NOT NULL REFERENCES complaints(id),
    action_taken        TEXT NOT NULL,
    root_cause_summary  TEXT,
    customer_acceptance TEXT NOT NULL
                            CHECK (customer_acceptance IN ('accepted','rejected','no_response','n_a')),
    customer_response_at TIMESTAMPTZ,
    resolved_by         TEXT NOT NULL,
    resolved_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, complaint_id)
);

CREATE INDEX IF NOT EXISTS idx_cc_resolutions_complaint
    ON complaint_resolutions (complaint_id);

-- ── Complaint Status Labels ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cc_status_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    canonical_status TEXT NOT NULL,
    display_label   TEXT NOT NULL,
    description     TEXT,
    updated_by      TEXT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, canonical_status)
);

-- ── Complaint Severity Labels ─────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cc_severity_labels (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    canonical_severity  TEXT NOT NULL,
    display_label       TEXT NOT NULL,
    description         TEXT,
    updated_by          TEXT NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, canonical_severity)
);

-- ── Complaint Source Labels ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cc_source_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    canonical_source TEXT NOT NULL,
    display_label   TEXT NOT NULL,
    description     TEXT,
    updated_by      TEXT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, canonical_source)
);

-- ── Complaint Category Codes ──────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS complaint_category_codes (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    category_code   TEXT NOT NULL,
    display_label   TEXT NOT NULL,
    description     TEXT,
    active          BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT NOT NULL,
    UNIQUE (tenant_id, category_code)
);

CREATE INDEX IF NOT EXISTS idx_cc_category_codes_tenant
    ON complaint_category_codes (tenant_id, active);

-- ── Outbox ────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cc_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    event_type      TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  TEXT,
    causation_id    TEXT,
    schema_version  TEXT NOT NULL DEFAULT '1.0.0',
    published       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_cc_outbox_unpublished
    ON cc_outbox (created_at) WHERE published = FALSE;

-- ── Processed Events ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cc_processed_events (
    id          BIGSERIAL PRIMARY KEY,
    event_id    UUID NOT NULL UNIQUE,
    event_type  TEXT NOT NULL,
    processor   TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_cc_processed_events_event_id
    ON cc_processed_events (event_id);
