-- Seed carrier sandbox credentials (bd-kvpda)
--
-- Reads credentials from session-local GUC settings (app.* namespace).
-- Each carrier INSERT is skipped with a NOTICE if the required settings
-- are absent or empty — the migration NEVER fails due to missing credentials.
--
-- Usage from CI (secrets injected as GUC settings via psql -c):
--
--   psql "$DATABASE_URL" \
--     -c "SET app.usps_user_id = '${USPS_USER_ID}';" \
--     -c "SET app.ups_client_id = '${UPS_CLIENT_ID}';" \
--     -c "SET app.ups_client_secret = '${UPS_CLIENT_SECRET}';" \
--     -c "SET app.ups_account_number = '${UPS_ACCOUNT_NUMBER}';" \
--     -c "SET app.fedex_client_id = '${FEDEX_CLIENT_ID}';" \
--     -c "SET app.fedex_client_secret = '${FEDEX_CLIENT_SECRET}';" \
--     -c "SET app.fedex_account_number = '${FEDEX_ACCOUNT_NUMBER}';" \
--     -f modules/integrations/db/migrations/20260409000013_seed_carrier_sandbox_credentials.sql
--
-- Tenant ID used for test records: test-carrier-sandbox
-- Connector type mirrors carrier_code used by the dispatch consumer.

DO $$
DECLARE
    v_app_id          TEXT := 'test-carrier-sandbox';

    -- USPS Web Tools
    v_usps_user_id    TEXT := current_setting('app.usps_user_id',    TRUE);

    -- UPS OAuth2
    v_ups_client_id      TEXT := current_setting('app.ups_client_id',      TRUE);
    v_ups_client_secret  TEXT := current_setting('app.ups_client_secret',  TRUE);
    v_ups_account_number TEXT := current_setting('app.ups_account_number', TRUE);

    -- FedEx REST API
    v_fedex_client_id      TEXT := current_setting('app.fedex_client_id',      TRUE);
    v_fedex_client_secret  TEXT := current_setting('app.fedex_client_secret',  TRUE);
    v_fedex_account_number TEXT := current_setting('app.fedex_account_number', TRUE);
BEGIN

    -- ── USPS ────────────────────────────────────────────────────────────────
    IF v_usps_user_id IS NOT NULL AND v_usps_user_id <> '' THEN
        INSERT INTO integrations_connector_configs
            (app_id, connector_type, name, config, enabled)
        VALUES (
            v_app_id,
            'usps',
            'USPS Web Tools Sandbox',
            jsonb_build_object('user_id', v_usps_user_id),
            TRUE
        )
        ON CONFLICT (app_id, connector_type, name)
        DO UPDATE SET
            config     = EXCLUDED.config,
            updated_at = NOW();
        RAISE NOTICE '[carrier-seed] Seeded USPS sandbox credentials for app_id=%', v_app_id;
    ELSE
        RAISE NOTICE '[carrier-seed] Skipping USPS: app.usps_user_id is not set';
    END IF;

    -- ── UPS ─────────────────────────────────────────────────────────────────
    IF v_ups_client_id IS NOT NULL AND v_ups_client_id <> ''
       AND v_ups_client_secret IS NOT NULL AND v_ups_client_secret <> ''
       AND v_ups_account_number IS NOT NULL AND v_ups_account_number <> ''
    THEN
        INSERT INTO integrations_connector_configs
            (app_id, connector_type, name, config, enabled)
        VALUES (
            v_app_id,
            'ups',
            'UPS OAuth2 Sandbox',
            jsonb_build_object(
                'client_id',      v_ups_client_id,
                'client_secret',  v_ups_client_secret,
                'account_number', v_ups_account_number
            ),
            TRUE
        )
        ON CONFLICT (app_id, connector_type, name)
        DO UPDATE SET
            config     = EXCLUDED.config,
            updated_at = NOW();
        RAISE NOTICE '[carrier-seed] Seeded UPS sandbox credentials for app_id=%', v_app_id;
    ELSE
        RAISE NOTICE '[carrier-seed] Skipping UPS: one or more of app.ups_client_id / ups_client_secret / ups_account_number not set';
    END IF;

    -- ── FedEx ────────────────────────────────────────────────────────────────
    IF v_fedex_client_id IS NOT NULL AND v_fedex_client_id <> ''
       AND v_fedex_client_secret IS NOT NULL AND v_fedex_client_secret <> ''
       AND v_fedex_account_number IS NOT NULL AND v_fedex_account_number <> ''
    THEN
        INSERT INTO integrations_connector_configs
            (app_id, connector_type, name, config, enabled)
        VALUES (
            v_app_id,
            'fedex',
            'FedEx REST API Sandbox',
            jsonb_build_object(
                'client_id',      v_fedex_client_id,
                'client_secret',  v_fedex_client_secret,
                'account_number', v_fedex_account_number
            ),
            TRUE
        )
        ON CONFLICT (app_id, connector_type, name)
        DO UPDATE SET
            config     = EXCLUDED.config,
            updated_at = NOW();
        RAISE NOTICE '[carrier-seed] Seeded FedEx sandbox credentials for app_id=%', v_app_id;
    ELSE
        RAISE NOTICE '[carrier-seed] Skipping FedEx: one or more of app.fedex_client_id / fedex_client_secret / fedex_account_number not set';
    END IF;

END $$;
