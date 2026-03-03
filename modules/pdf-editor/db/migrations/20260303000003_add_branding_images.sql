-- Branding images: tenant-scoped logo/image storage for PDF injection.
-- Supports header logos, footer branding, and inline images.

CREATE TABLE branding_images (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    name            TEXT NOT NULL,
    image_format    TEXT NOT NULL
                    CHECK (image_format IN ('png', 'jpeg', 'svg')),
    width_px        INTEGER,
    height_px       INTEGER,
    size_bytes      BIGINT NOT NULL,
    image_data      BYTEA NOT NULL,
    placement       TEXT NOT NULL
                    CHECK (placement IN ('header_logo', 'footer_branding', 'inline')),
    created_by      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT branding_images_idempotency UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_branding_images_tenant ON branding_images(tenant_id);
CREATE INDEX idx_branding_images_tenant_placement ON branding_images(tenant_id, placement);
