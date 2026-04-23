-- Admin-managed carrier credentials (encrypted).
-- Supplements integrations_connector_configs (CI-seeded sandbox creds).
-- get_carrier_credentials falls back to connector_configs when no row exists here.

CREATE TABLE integrations_carrier_credentials (
    app_id          TEXT        NOT NULL,
    carrier_type    TEXT        NOT NULL
        CHECK (carrier_type IN ('ups', 'fedex', 'usps')),
    creds_enc       BYTEA       NOT NULL,
    configured_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_id, carrier_type)
);
