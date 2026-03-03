-- Workforce Competence: Core schema
--
-- Two primary tables:
-- 1. wc_competence_artifacts — the registry of skills/certs/qualifications
-- 2. wc_operator_competences — assignments of competence to operators
--
-- Design: Never delete rows. Revocations set is_revoked + revoked_at for audit trail.
-- Authorization queries are time-aware: awarded_at <= T AND (expires_at IS NULL OR expires_at > T).

-- Competence artifact registry (the "what")
CREATE TABLE wc_competence_artifacts (
    id                   UUID PRIMARY KEY,
    tenant_id            TEXT NOT NULL,
    artifact_type        TEXT NOT NULL CHECK (artifact_type IN ('certification', 'training', 'qualification')),
    name                 TEXT NOT NULL,
    code                 TEXT NOT NULL,
    description          TEXT,
    valid_duration_days  INT,
    is_active            BOOLEAN NOT NULL DEFAULT true,
    created_at           TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT wc_artifacts_tenant_code_unique UNIQUE (tenant_id, code)
);

CREATE INDEX idx_wc_artifacts_tenant ON wc_competence_artifacts(tenant_id);
CREATE INDEX idx_wc_artifacts_code   ON wc_competence_artifacts(tenant_id, code);

-- Operator competence assignments (the "who has what")
CREATE TABLE wc_operator_competences (
    id               UUID PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    operator_id      UUID NOT NULL,
    artifact_id      UUID NOT NULL REFERENCES wc_competence_artifacts(id),
    awarded_at       TIMESTAMP WITH TIME ZONE NOT NULL,
    expires_at       TIMESTAMP WITH TIME ZONE,
    evidence_ref     TEXT,
    awarded_by       TEXT,
    is_revoked       BOOLEAN NOT NULL DEFAULT false,
    revoked_at       TIMESTAMP WITH TIME ZONE,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    idempotency_key  TEXT NOT NULL,

    CONSTRAINT wc_operator_competences_idem_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_wc_oc_tenant_operator ON wc_operator_competences(tenant_id, operator_id);
CREATE INDEX idx_wc_oc_artifact        ON wc_operator_competences(artifact_id);
CREATE INDEX idx_wc_oc_expires         ON wc_operator_competences(expires_at) WHERE expires_at IS NOT NULL;

-- Authorization query index: tenant + operator + not revoked + time range
CREATE INDEX idx_wc_oc_auth_lookup ON wc_operator_competences(tenant_id, operator_id, artifact_id)
    WHERE is_revoked = false;

-- Transactional outbox
CREATE TABLE wc_outbox (
    id               BIGSERIAL PRIMARY KEY,
    event_id         UUID NOT NULL UNIQUE,
    event_type       TEXT NOT NULL,
    aggregate_type   TEXT NOT NULL,
    aggregate_id     TEXT NOT NULL,
    tenant_id        TEXT NOT NULL,
    payload          JSONB NOT NULL,
    correlation_id   TEXT,
    causation_id     TEXT,
    schema_version   TEXT NOT NULL DEFAULT '1.0.0',
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    published_at     TIMESTAMP WITH TIME ZONE
);

CREATE INDEX idx_wc_outbox_unpublished ON wc_outbox(created_at) WHERE published_at IS NULL;
CREATE INDEX idx_wc_outbox_tenant      ON wc_outbox(tenant_id);

-- Idempotent consumer tracking
CREATE TABLE wc_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_wc_processed_event_id ON wc_processed_events(event_id);

-- HTTP idempotency keys (scoped per tenant)
CREATE TABLE wc_idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      INT NOT NULL,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMP WITH TIME ZONE NOT NULL,

    CONSTRAINT wc_idempotency_keys_tenant_key_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_wc_idempotency_expires ON wc_idempotency_keys(expires_at);
CREATE INDEX idx_wc_idempotency_tenant  ON wc_idempotency_keys(tenant_id);
