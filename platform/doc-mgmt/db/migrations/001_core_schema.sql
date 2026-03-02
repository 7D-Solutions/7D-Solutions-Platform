-- Core document management schema: documents, revisions, outbox, idempotency.
--
-- Lifecycle: draft → released (baseline).
-- Every mutation writes to the outbox for event emission.
-- Idempotency keys prevent duplicate creates on retry.

CREATE TABLE IF NOT EXISTS documents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    doc_number      VARCHAR(128) NOT NULL,
    title           VARCHAR(512) NOT NULL,
    doc_type        VARCHAR(64) NOT NULL,           -- e.g. "work_order", "purchase_order", "spec"
    status          VARCHAR(32) NOT NULL DEFAULT 'draft',  -- draft | released
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (tenant_id, doc_number)
);

CREATE INDEX idx_documents_tenant ON documents (tenant_id);
CREATE INDEX idx_documents_status ON documents (tenant_id, status);

CREATE TABLE IF NOT EXISTS revisions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id     UUID NOT NULL REFERENCES documents(id),
    tenant_id       UUID NOT NULL,
    revision_number INTEGER NOT NULL DEFAULT 1,
    body            JSONB NOT NULL DEFAULT '{}',
    change_summary  TEXT NOT NULL DEFAULT '',
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (document_id, revision_number)
);

CREATE INDEX idx_revisions_document ON revisions (document_id);

-- Outbox for reliable event publishing (Guard → Mutation → Outbox).
CREATE TABLE IF NOT EXISTS doc_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_type      VARCHAR(128) NOT NULL,
    subject         VARCHAR(256) NOT NULL,          -- NATS subject
    payload         JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at    TIMESTAMPTZ                     -- NULL until relay publishes
);

CREATE INDEX idx_doc_outbox_unpublished ON doc_outbox (created_at) WHERE published_at IS NULL;

-- Idempotency key store (per platform-contracts convention).
CREATE TABLE IF NOT EXISTS doc_idempotency_keys (
    id              SERIAL PRIMARY KEY,
    app_id          VARCHAR(255) NOT NULL,
    idempotency_key VARCHAR(512) NOT NULL,
    request_hash    VARCHAR(64),
    response_body   JSONB NOT NULL,
    status_code     INT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    UNIQUE (app_id, idempotency_key)
);
