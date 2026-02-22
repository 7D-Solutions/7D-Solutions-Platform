/**
 * Environment configuration for k6 performance tests.
 *
 * Controls which base URLs each service resolves to.
 *
 * Selection order (highest priority first):
 *   1. Per-service URL env vars (PERF_AUTH_URL, PERF_CONTROL_PLANE_URL, etc.)
 *   2. PERF_ENV preset (local | staging)
 *   3. Default: local
 *
 * Local ports match docker-compose.platform.yml / docker-compose.modules.yml.
 * Staging ports are the same unless STAGING_HOST is set with custom mappings.
 */

/* global __ENV */

const stagingHost = __ENV.STAGING_HOST || '';

const PRESETS = {
  local: {
    auth:         'http://localhost:8080',
    controlPlane: 'http://localhost:8091',
    ar:           'http://localhost:8086',
    ttp:          'http://localhost:8100',
    payments:     'http://localhost:8088',
  },
  staging: {
    auth:         `http://${stagingHost}:8080`,
    controlPlane: `http://${stagingHost}:8091`,
    ar:           `http://${stagingHost}:8086`,
    ttp:          `http://${stagingHost}:8100`,
    payments:     `http://${stagingHost}:8088`,
  },
};

const envName = __ENV.PERF_ENV || 'local';
if (!PRESETS[envName]) {
  throw new Error(
    `Unknown PERF_ENV "${envName}". Valid values: ${Object.keys(PRESETS).join(', ')}`
  );
}

const preset = PRESETS[envName];

/**
 * Resolved base URLs, after applying per-service overrides.
 * Import this in test scripts.
 */
export const urls = {
  auth:         __ENV.PERF_AUTH_URL          || preset.auth,
  controlPlane: __ENV.PERF_CONTROL_PLANE_URL || preset.controlPlane,
  ar:           __ENV.PERF_AR_URL            || preset.ar,
  ttp:          __ENV.PERF_TTP_URL           || preset.ttp,
  payments:     __ENV.PERF_PAYMENTS_URL      || preset.payments,
};

/**
 * Auth credentials for token acquisition.
 * PERF_AUTH_TOKEN bypasses login entirely (use a pre-minted JWT).
 */
export const credentials = {
  tenantId: __ENV.PERF_TENANT_ID    || '00000000-0000-0000-0000-000000000000',
  email:    __ENV.PERF_AUTH_EMAIL   || '',
  password: __ENV.PERF_AUTH_PASSWORD || '',
  token:    __ENV.PERF_AUTH_TOKEN   || '',
};
