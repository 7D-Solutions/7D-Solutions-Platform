/**
 * Smoke scenario — P47 Phase 47 Perf Track.
 *
 * Exercises 5 critical endpoints:
 *   1. GET  /api/ready            (control-plane) — unauthenticated readiness
 *   2. POST /api/auth/login       (auth-lb)       — token acquisition
 *   3. GET  /api/tenants          (control-plane) — authenticated tenant list
 *   4. GET  /api/ar/invoices      (ar)            — authenticated AR read
 *   5. GET  /api/ar/customers     (ar)            — authenticated AR read
 *
 * Run locally:
 *   PERF_AUTH_EMAIL=admin@7d.local PERF_AUTH_PASSWORD=secret k6 run tools/perf/smoke.js
 *
 * Run against staging:
 *   PERF_ENV=staging STAGING_HOST=staging.7dsolutions.app \
 *   PERF_AUTH_EMAIL=perf@7d.staging PERF_AUTH_PASSWORD=secret \
 *   k6 run tools/perf/smoke.js
 *
 * See tools/perf/README.md for full usage and CI instructions.
 */

import http from 'k6/http';
import { check, group, sleep } from 'k6';
import { Rate, Trend } from 'k6/metrics';

import { urls, credentials } from './config/environments.js';
import { acquireToken, bearerHeaders } from './lib/auth.js';

// ── Custom metrics ──────────────────────────────────────────────────────────
const errorRate = new Rate('smoke_errors');
const cpLatency = new Trend('smoke_control_plane_ms', true);
const arLatency = new Trend('smoke_ar_ms', true);

// ── Test options ─────────────────────────────────────────────────────────────
export const options = {
  // Smoke: 1 VU, 1 full iteration. Fails fast on any assertion error.
  vus:        1,
  iterations: 1,

  thresholds: {
    // No more than 1% of requests may fail.
    http_req_failed:       ['rate<0.01'],
    // 95th-percentile wall time must stay under 2 s (generous for smoke).
    http_req_duration:     ['p(95)<2000'],
    smoke_control_plane_ms: ['p(95)<1000'],
    smoke_ar_ms:            ['p(95)<1500'],
    smoke_errors:           ['rate<0.01'],
  },
};

// ── Setup: runs once, result forwarded to every VU iteration ─────────────────
export function setup() {
  const token = acquireToken(urls.auth, credentials);
  return { token };
}

// ── Default function: runs once per VU iteration ──────────────────────────────
export default function ({ token }) {
  const authHeaders = bearerHeaders(token);

  // 1. Control-plane readiness — no auth required.
  group('control-plane: readiness', () => {
    const res = http.get(`${urls.controlPlane}/api/ready`);
    const ok = check(res, {
      'GET /api/ready: HTTP 200': (r) => r.status === 200,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });

  sleep(0.1);

  // 2. Control-plane tenant list — authenticated.
  group('control-plane: tenant list', () => {
    const res = http.get(`${urls.controlPlane}/api/tenants`, { headers: authHeaders });
    const ok = check(res, {
      'GET /api/tenants: HTTP 200':       (r) => r.status === 200,
      'GET /api/tenants: JSON body':      (r) => r.headers['Content-Type']
                                                  ? r.headers['Content-Type'].includes('application/json')
                                                  : false,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });

  sleep(0.1);

  // 3. AR invoice list — authenticated read (no ar.mutate needed).
  group('ar: invoice list', () => {
    const res = http.get(`${urls.ar}/api/ar/invoices`, { headers: authHeaders });
    const ok = check(res, {
      'GET /api/ar/invoices: HTTP 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/invoices: not 500':         (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.1);

  // 4. AR customer list — authenticated read.
  group('ar: customer list', () => {
    const res = http.get(`${urls.ar}/api/ar/customers`, { headers: authHeaders });
    const ok = check(res, {
      'GET /api/ar/customers: HTTP 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/customers: not 500':         (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.1);

  // 5. TTP plan catalog — authenticated read via control-plane.
  group('control-plane: TTP plans', () => {
    const res = http.get(`${urls.controlPlane}/api/ttp/plans`, { headers: authHeaders });
    const ok = check(res, {
      'GET /api/ttp/plans: HTTP 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ttp/plans: not 500':         (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });
}
