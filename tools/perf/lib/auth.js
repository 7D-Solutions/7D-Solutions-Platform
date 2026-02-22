/**
 * Token acquisition helpers for k6 perf scenarios.
 *
 * Usage — in k6 setup():
 *   import { acquireToken } from './lib/auth.js';
 *   export function setup() {
 *     return { token: acquireToken(urls.auth, credentials) };
 *   }
 */

import http from 'k6/http';
import { check } from 'k6';

/**
 * Acquire a bearer token from identity-auth.
 *
 * If credentials.token is already set, it is returned immediately without
 * making an HTTP call (useful when you have a pre-minted token, e.g. from
 * staging seed scripts or CI secrets).
 *
 * @param {string} authUrl  - Base URL of identity-auth (e.g. http://localhost:8080)
 * @param {object} credentials
 * @param {string} credentials.tenantId
 * @param {string} credentials.email
 * @param {string} credentials.password
 * @param {string} credentials.token  - Optional pre-minted JWT; skips login if set.
 * @returns {string} Bearer token (without "Bearer " prefix)
 */
export function acquireToken(authUrl, credentials) {
  if (credentials.token) {
    console.log('auth: using pre-minted PERF_AUTH_TOKEN');
    return credentials.token;
  }

  if (!credentials.email || !credentials.password) {
    throw new Error(
      'Provide PERF_AUTH_EMAIL + PERF_AUTH_PASSWORD (or PERF_AUTH_TOKEN) to authenticate.'
    );
  }

  const res = http.post(
    `${authUrl}/api/auth/login`,
    JSON.stringify({
      tenant_id: credentials.tenantId,
      email:     credentials.email,
      password:  credentials.password,
    }),
    { headers: { 'Content-Type': 'application/json' } }
  );

  check(res, { 'auth/login: HTTP 200': (r) => r.status === 200 });

  if (res.status !== 200) {
    throw new Error(`Token acquisition failed: HTTP ${res.status} — ${res.body}`);
  }

  let body;
  try {
    body = JSON.parse(res.body);
  } catch (_) {
    throw new Error(`Login response is not JSON: ${res.body}`);
  }

  if (!body.access_token) {
    throw new Error(`Login response missing access_token: ${res.body}`);
  }

  console.log(`auth: token acquired (${body.access_token.length} chars)`);
  return body.access_token;
}

/**
 * Return an Authorization header object for the given bearer token.
 */
export function bearerHeaders(token) {
  return { Authorization: `Bearer ${token}` };
}
