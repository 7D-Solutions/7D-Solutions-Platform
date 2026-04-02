-- cp_service_catalog: module-to-URL mapping for service discovery
--
-- Replaces hardcoded env vars (AR_BASE_URL, TENANT_REGISTRY_URL, DOC_MGMT_BASE_URL)
-- with a single queryable table. Verticals query GET /api/service-catalog instead
-- of configuring N env vars per module.

CREATE TABLE IF NOT EXISTS cp_service_catalog (
    module_code  TEXT        PRIMARY KEY,
    base_url     TEXT        NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE  cp_service_catalog              IS 'Module-to-URL mappings for service discovery; queried via GET /api/service-catalog';
COMMENT ON COLUMN cp_service_catalog.module_code   IS 'Module identifier matching cp_bundle_modules.module_code (e.g. ar, gl, inventory)';
COMMENT ON COLUMN cp_service_catalog.base_url      IS 'HTTP base URL for the module (e.g. http://7d-ar:8086)';
COMMENT ON COLUMN cp_service_catalog.updated_at    IS 'Last time this entry was updated';

-- Seed with all known platform modules and their Docker-internal URLs
INSERT INTO cp_service_catalog (module_code, base_url) VALUES
    ('control-plane',         'http://7d-control-plane:8091'),
    ('ar',                    'http://7d-ar:8086'),
    ('subscriptions',         'http://7d-subscriptions:8087'),
    ('payments',              'http://7d-payments:8088'),
    ('notifications',         'http://7d-notifications:8089'),
    ('gl',                    'http://7d-gl:8090'),
    ('inventory',             'http://7d-inventory:8092'),
    ('ap',                    'http://7d-ap:8093'),
    ('treasury',              'http://7d-treasury:8094'),
    ('fixed-assets',          'http://7d-fixed-assets:8104'),
    ('consolidation',         'http://7d-consolidation:8105'),
    ('timekeeping',           'http://7d-timekeeping:8097'),
    ('party',                 'http://7d-party:8098'),
    ('integrations',          'http://7d-integrations:8099'),
    ('ttp',                   'http://7d-ttp:8100'),
    ('maintenance',           'http://7d-maintenance:8101'),
    ('pdf-editor',            'http://7d-pdf-editor:8102'),
    ('shipping-receiving',    'http://7d-shipping-receiving:8103'),
    ('quality-inspection',    'http://7d-quality-inspection:8106'),
    ('bom',                   'http://7d-bom:8107'),
    ('production',            'http://7d-production:8108'),
    ('workflow',              'http://7d-workflow:8110'),
    ('numbering',             'http://7d-numbering:8120'),
    ('workforce-competence',  'http://7d-workforce-competence:8121'),
    ('customer-portal',       'http://7d-customer-portal:8111'),
    ('reporting',             'http://7d-reporting:8096')
ON CONFLICT (module_code) DO NOTHING;
