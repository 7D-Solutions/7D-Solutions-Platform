/**
 * Baseline capacity scenario — billing spine.
 *
 * Exercises the read-heavy, write-light operations that form the core billing
 * spine: control-plane reads, tenant registry lookups, AR invoice/customer
 * lifecycle reads, and the TTP plan catalog.  No real charges are created.
 *
 * Load shape:
 *   - Ramp from 1 to 10 VUs over 30 s
 *   - Sustain 10 VUs for 60 s
 *   - Ramp down to 0 VUs over 10 s
 *
 * Run locally (Docker Compose stack must be up):
 *   PERF_AUTH_EMAIL=perf@test.7d.local \
 *   PERF_AUTH_PASSWORD='PerfTest1!' \
 *   k6 run tools/perf/baseline_billing_spine.js \
 *        --summary-export=perf_summary.json
 *
 * Run against staging:
 *   PERF_ENV=staging STAGING_HOST=staging.7dsolutions.app \
 *   PERF_AUTH_EMAIL=perf@staging.7d.internal \
 *   PERF_AUTH_PASSWORD='StrongPass1!' \
 *   k6 run tools/perf/baseline_billing_spine.js \
 *        --summary-export=perf_summary.json
 *
 * See tools/perf/README.md — "Baseline" section for pass/fail criteria.
 */

import http from 'k6/http';
import { check, group, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';

import { urls, credentials } from './config/environments.js';
import { acquireToken, bearerHeaders } from './lib/auth.js';

// ── Custom metrics ─────────────────────────────────────────────────────────────
const errorRate   = new Rate('billing_errors');
const cpLatency   = new Trend('billing_cp_reads_ms',  true);  // control-plane
const arLatency   = new Trend('billing_ar_reads_ms',  true);  // AR module
const writeOps    = new Counter('billing_write_ops');          // write-light count

// ── Test options ───────────────────────────────────────────────────────────────
export const options = {
  stages: [
    { duration: '30s', target: 10 },  // ramp up
    { duration: '60s', target: 10 },  // sustain
    { duration: '10s', target: 0  },  // ramp down
  ],

  thresholds: {
    // Overall request failure rate must stay under 1%.
    http_req_failed:      ['rate<0.01'],

    // Wall-clock p95 across all requests.
    http_req_duration:    ['p(95)<1000'],

    // Service-level thresholds — controls the pass/fail gate per tier.
    billing_cp_reads_ms:  ['p(95)<500'],
    billing_ar_reads_ms:  ['p(95)<800'],

    // Custom error counter (based on check() failures, not HTTP errors).
    billing_errors:       ['rate<0.01'],
  },
};

// ── Setup: runs once before any VU starts ─────────────────────────────────────
export function setup() {
  const token = acquireToken(urls.auth, credentials);
  return { token };
}

// ── Default function: executes once per VU per iteration ──────────────────────
export default function ({ token }) {
  const auth = bearerHeaders(token);

  // ── Read group 1: control-plane infrastructure ────────────────────────────
  group('cp: readiness', () => {
    const res = http.get(`${urls.controlPlane}/api/ready`);
    const ok = check(res, {
      'GET /api/ready: 200': (r) => r.status === 200,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 2: tenant registry list ───────────────────────────────────
  group('cp: tenant list', () => {
    const res = http.get(
      `${urls.controlPlane}/api/tenants`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/tenants: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/tenants: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 3: TTP plan catalog ───────────────────────────────────────
  group('cp: ttp plan catalog', () => {
    const res = http.get(
      `${urls.controlPlane}/api/ttp/plans`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/ttp/plans: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ttp/plans: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 4: AR customer list ───────────────────────────────────────
  group('ar: customer list', () => {
    const res = http.get(
      `${urls.ar}/api/ar/customers`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/ar/customers: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/customers: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 5: AR invoice list ────────────────────────────────────────
  group('ar: invoice list', () => {
    const res = http.get(
      `${urls.ar}/api/ar/invoices`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/ar/invoices: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/invoices: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 6: AR subscription list ───────────────────────────────────
  group('ar: subscription list', () => {
    const res = http.get(
      `${urls.ar}/api/ar/subscriptions`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/ar/subscriptions: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/subscriptions: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Read group 7: AR aging report ────────────────────────────────────────
  // Aging is heavier (aggregates across invoices) — good stress test for DB.
  group('ar: aging report', () => {
    const res = http.get(
      `${urls.ar}/api/ar/aging`,
      { headers: auth },
    );
    const ok = check(res, {
      'GET /api/ar/aging: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/aging: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });

  sleep(0.05);

  // ── Write-light: safe customer create ────────────────────────────────────
  // Runs on ~20% of VU iterations to simulate realistic mixed traffic.
  // Creates a customer record — no payment method, no invoice, no real charge.
  if (Math.random() < 0.2) {
    group('ar: write-light customer create', () => {
      const ts   = Date.now();
      const body = JSON.stringify({
        name:  `Perf Baseline ${ts}`,
        email: `perf-${ts}@baseline.test`,
      });
      const res = http.post(
        `${urls.ar}/api/ar/customers`,
        body,
        { headers: { ...auth, 'Content-Type': 'application/json' } },
      );
      const ok = check(res, {
        'POST /api/ar/customers: 201 or 401': (r) => r.status === 201 || r.status === 401,
        'POST /api/ar/customers: not 500':    (r) => r.status !== 500,
      });
      errorRate.add(!ok);
      arLatency.add(res.timings.duration);
      writeOps.add(1);
    });
    sleep(0.1);
  }
}

// ── Teardown: runs once after all VUs finish ──────────────────────────────────
// Nothing to clean up — customer records from write-light iterations are
// low-volume test data; production stacks should point to a staging DB.
export function teardown() {
  console.log('baseline_billing_spine: run complete');
}
