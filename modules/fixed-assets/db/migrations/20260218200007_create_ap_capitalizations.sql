-- Fixed Assets: AP Capitalization Linkage
-- bd-vo7u: Records assets created from AP bill approval events.
--
-- fa_ap_capitalizations:
--   Links each AP bill line that was capitalized to the resulting fa_asset.
--   Idempotency key: (tenant_id, bill_id, line_id) — prevents duplicate assets
--   on event replay.
--
-- No cross-module FK references — bill_id / line_id are soft references only.
-- source_ref = "{bill_id}:{line_id}" for human-readable audit trail.

CREATE TABLE fa_ap_capitalizations (
    id              BIGSERIAL       PRIMARY KEY,
    tenant_id       TEXT            NOT NULL,
    bill_id         UUID            NOT NULL,
    line_id         UUID            NOT NULL,
    asset_id        UUID            NOT NULL REFERENCES fa_assets(id),
    gl_account_code TEXT            NOT NULL,
    amount_minor    BIGINT          NOT NULL CHECK (amount_minor >= 0),
    currency        TEXT            NOT NULL,
    -- Human-readable audit reference: "{bill_id}:{line_id}"
    source_ref      TEXT            NOT NULL,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    CONSTRAINT fa_ap_capitalizations_bill_line_unique
        UNIQUE (tenant_id, bill_id, line_id)
);

CREATE INDEX idx_fa_ap_cap_tenant    ON fa_ap_capitalizations (tenant_id);
CREATE INDEX idx_fa_ap_cap_bill      ON fa_ap_capitalizations (bill_id);
CREATE INDEX idx_fa_ap_cap_asset     ON fa_ap_capitalizations (asset_id);
