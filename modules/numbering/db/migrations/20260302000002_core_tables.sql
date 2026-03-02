-- Numbering core tables: sequences, issued numbers, idempotency dedup

-- Sequences: one row per tenant+entity, holds the current counter value.
-- SELECT FOR UPDATE serialises concurrent allocations.
CREATE TABLE sequences (
    tenant_id UUID NOT NULL,
    entity    VARCHAR(100) NOT NULL,
    current_value BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, entity)
);

-- Issued numbers: audit log of every allocation.
CREATE TABLE issued_numbers (
    id            SERIAL PRIMARY KEY,
    tenant_id     UUID NOT NULL,
    entity        VARCHAR(100) NOT NULL,
    number_value  BIGINT NOT NULL,
    idempotency_key VARCHAR(512) NOT NULL,
    created_at    TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_issued_numbers_entity ON issued_numbers (tenant_id, entity, number_value);
