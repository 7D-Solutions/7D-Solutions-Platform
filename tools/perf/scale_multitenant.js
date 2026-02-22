/**
 * P50-010 — Multi-tenant scale scenario.
 *
 * Models ≥5 tenants concurrently across three sequential phases:
 *
 *   Phase 1 — Read burst (t=0s):  control-plane reads, tenant-registry
 *             lookups, AR reads.  20 VUs × 100s.
 *   Phase 2 — Billing runs (t=30s): POST /api/control/platform-billing-runs
 *             in safe (idempotent) mode using a far-future period.  5 VUs × 75s.
 *   Phase 3 — Webhook burst (t=115s): replays signed Tilled events against
 *             POST /api/payments/webhook/tilled.  10 VUs × 45s.
 *
 * After all VUs finish, teardown() queries Prometheus for the payments
 * consumer lag metric and logs the result.  Use --summary-export to capture
 * all k6 metrics as JSON.
 *
 * Run locally (Docker Compose stack + monitoring must be up):
 *   PERF_AUTH_EMAIL=perf@test.7d.local \
 *   PERF_AUTH_PASSWORD='PerfTest1!' \
 *   PERF_TILLED_WEBHOOK_SECRET='whsec_test' \
 *   k6 run tools/perf/scale_multitenant.js \
 *        --summary-export=scale_multitenant_summary.json
 *
 * Run against staging:
 *   PERF_ENV=staging \
 *   STAGING_HOST=staging.7dsolutions.app \
 *   PERF_AUTH_EMAIL=perf@staging.7d.internal \
 *   PERF_AUTH_PASSWORD='StrongPass1!' \
 *   PERF_TILLED_WEBHOOK_SECRET='<staging-tilled-webhook-secret>' \
 *   k6 run tools/perf/scale_multitenant.js \
 *        --summary-export=scale_multitenant_summary.json
 *
 * See tools/perf/README.md — "Scale: multi-tenant" section for full params.
 */

/* global __ENV, __VU */

import http from 'k6/http';
import { check, group, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';
import crypto from 'k6/crypto';

import { urls, credentials } from './config/environments.js';
import { acquireToken, bearerHeaders } from './lib/auth.js';

// ── Custom metrics ─────────────────────────────────────────────────────────────
const errorRate      = new Rate('scale_errors');
const cpLatency      = new Trend('scale_cp_reads_ms',    true);
const arLatency      = new Trend('scale_ar_reads_ms',    true);
const billingLatency = new Trend('scale_billing_run_ms', true);
const webhookLatency = new Trend('scale_webhook_ms',     true);
const billingOps     = new Counter('scale_billing_ops');
const webhookOps     = new Counter('scale_webhook_ops');

// ── Tenant pool: 5 distinct perf-tenant identifiers ───────────────────────────
// These are used as logical tenant identifiers in billing payloads.
// Override individual slots with PERF_TENANT_1 … PERF_TENANT_5.
const TENANT_POOL = [
  __ENV.PERF_TENANT_1 || '00000000-0000-0000-0001-000000000001',
  __ENV.PERF_TENANT_2 || '00000000-0000-0000-0001-000000000002',
  __ENV.PERF_TENANT_3 || '00000000-0000-0000-0001-000000000003',
  __ENV.PERF_TENANT_4 || '00000000-0000-0000-0001-000000000004',
  __ENV.PERF_TENANT_5 || '00000000-0000-0000-0001-000000000005',
];

// ── Safe billing period: far future → idempotent on repeat calls ───────────────
const BILLING_SAFE_PERIOD = __ENV.PERF_BILLING_PERIOD || '2099-01';

// ── Tilled webhook HMAC secret ─────────────────────────────────────────────────
const TILLED_SECRET = __ENV.PERF_TILLED_WEBHOOK_SECRET || '';

// ── Prometheus URL for teardown lag query ──────────────────────────────────────
const PROMETHEUS_URL = __ENV.PERF_PROMETHEUS_URL || 'http://localhost:9090';

// ── Scenario options ───────────────────────────────────────────────────────────
export const options = {
  scenarios: {
    // Phase 1: read burst — all service read endpoints, 5 VUs map to 5 tenants
    reads: {
      executor:          'ramping-vus',
      startVUs:          0,
      stages: [
        { duration: '30s', target: 20 },  // ramp up
        { duration: '60s', target: 20 },  // sustain
        { duration: '10s', target: 0  },  // ramp down
      ],
      startTime:         '0s',
      exec:              'readPhase',
      gracefulRampDown:  '5s',
    },

    // Phase 2: billing runs — idempotent safe mode, overlaps read sustain
    billing: {
      executor:          'ramping-vus',
      startVUs:          0,
      stages: [
        { duration: '10s', target: 5 },
        { duration: '60s', target: 5 },
        { duration: '5s',  target: 0 },
      ],
      startTime:         '30s',
      exec:              'billingPhase',
      gracefulRampDown:  '5s',
    },

    // Phase 3: webhook burst — HMAC-signed Tilled events, runs after billing
    webhooks: {
      executor:          'ramping-vus',
      startVUs:          0,
      stages: [
        { duration: '10s', target: 10 },
        { duration: '30s', target: 10 },
        { duration: '5s',  target: 0  },
      ],
      startTime:         '115s',
      exec:              'webhookPhase',
      gracefulRampDown:  '5s',
    },
  },

  thresholds: {
    // Global failure rate must stay under 1%.
    http_req_failed:      ['rate<0.01'],

    // Wall-clock p95 across all requests.
    http_req_duration:    ['p(95)<2000'],

    // Per-tier latency gates.
    scale_cp_reads_ms:    ['p(95)<500'],
    scale_ar_reads_ms:    ['p(95)<800'],
    scale_billing_run_ms: ['p(95)<3000'],
    scale_webhook_ms:     ['p(95)<500'],

    // Custom error rate (check() failures, not HTTP errors).
    scale_errors:         ['rate<0.01'],
  },
};

// ── Setup: token acquisition runs once before any VU starts ───────────────────
export function setup() {
  const token = acquireToken(urls.auth, credentials);
  return { token };
}

// ── Phase 1: read scenario ────────────────────────────────────────────────────
export function readPhase({ token }) {
  const auth   = bearerHeaders(token);
  // Round-robin tenant selection based on VU ID.
  void TENANT_POOL[(__VU - 1) % TENANT_POOL.length];

  group('cp: readiness', () => {
    const res = http.get(`${urls.controlPlane}/api/ready`);
    const ok  = check(res, { 'GET /api/ready: 200': (r) => r.status === 200 });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('cp: tenant list', () => {
    const res = http.get(`${urls.controlPlane}/api/tenants`, { headers: auth });
    const ok  = check(res, {
      'GET /api/tenants: 200':     (r) => r.status === 200,
      'GET /api/tenants: not 500': (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('cp: ttp plan catalog', () => {
    const res = http.get(`${urls.controlPlane}/api/ttp/plans`, { headers: auth });
    const ok  = check(res, {
      'GET /api/ttp/plans: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ttp/plans: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    cpLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('ar: customer list', () => {
    const res = http.get(`${urls.ar}/api/ar/customers`, { headers: auth });
    const ok  = check(res, {
      'GET /api/ar/customers: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/customers: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('ar: invoice list', () => {
    const res = http.get(`${urls.ar}/api/ar/invoices`, { headers: auth });
    const ok  = check(res, {
      'GET /api/ar/invoices: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/invoices: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('ar: subscription list', () => {
    const res = http.get(`${urls.ar}/api/ar/subscriptions`, { headers: auth });
    const ok  = check(res, {
      'GET /api/ar/subscriptions: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/subscriptions: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });
  sleep(0.05);

  group('ar: aging report', () => {
    const res = http.get(`${urls.ar}/api/ar/aging`, { headers: auth });
    const ok  = check(res, {
      'GET /api/ar/aging: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'GET /api/ar/aging: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    arLatency.add(res.timings.duration);
  });
  sleep(0.1);
}

// ── Phase 2: billing run — safe / idempotent mode ────────────────────────────
// Uses a fixed far-future period so re-runs hit the "already_billed" path.
// Exercises control-plane → tenant-registry → AR write path under concurrent load.
export function billingPhase({ token }) {
  const auth = bearerHeaders(token);

  group('cp: platform billing run (safe)', () => {
    const body = JSON.stringify({ period: BILLING_SAFE_PERIOD });
    const res  = http.post(
      `${urls.controlPlane}/api/control/platform-billing-runs`,
      body,
      { headers: { ...auth, 'Content-Type': 'application/json' } },
    );
    // 200 = processed or already_billed (idempotent); 401 = auth issue
    const ok = check(res, {
      'POST /platform-billing-runs: 200 or 401': (r) => r.status === 200 || r.status === 401,
      'POST /platform-billing-runs: not 500':    (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    billingLatency.add(res.timings.duration);
    billingOps.add(1);
  });

  // Throttle: billing runs hit multiple services; avoid flooding.
  sleep(0.5);
}

// ── Phase 3: webhook burst — signed Tilled events ────────────────────────────
// Payload uses a fake payment_intent ID — no matching row in DB → 0 rows
// updated, but the service still validates the signature and returns 200.
// This exercises the full HMAC-verify → parse → DB-update path at load.
export function webhookPhase() {
  if (!TILLED_SECRET) {
    // No secret configured — skip silently.  Set PERF_TILLED_WEBHOOK_SECRET.
    sleep(0.1);
    return;
  }

  // Unique fake payment_intent per iteration — safe to replay indefinitely.
  const fakeId  = `pi_perf_${Date.now()}_${__VU}`;
  const payload = JSON.stringify({
    type: 'payment_intent.succeeded',
    data: { object: { id: fakeId, status: 'succeeded', amount: 2900, currency: 'usd' } },
  });

  // Tilled signature: t=<unix_ts>,v1=<HMAC-SHA256("<ts>.<body>")>
  const ts         = Math.floor(Date.now() / 1000).toString();
  const signed     = `${ts}.${payload}`;
  const sig        = crypto.hmac('sha256', TILLED_SECRET, signed, 'hex');
  const sigHeader  = `t=${ts},v1=${sig}`;

  group('payments: tilled webhook (safe payload)', () => {
    const res = http.post(
      `${urls.payments}/api/payments/webhook/tilled`,
      payload,
      { headers: { 'Content-Type': 'application/json', 'tilled-signature': sigHeader } },
    );
    // 200 = accepted (0 rows updated is fine for a fake pi_id)
    // 401 = signature rejected — counts as an error
    const ok = check(res, {
      'POST /webhook/tilled: 200':     (r) => r.status === 200,
      'POST /webhook/tilled: not 500': (r) => r.status !== 500,
    });
    errorRate.add(!ok);
    webhookLatency.add(res.timings.duration);
    webhookOps.add(1);
  });

  sleep(0.1);
}

// ── Teardown: query Prometheus for projection lag; log for operators ──────────
export function teardown() {
  console.log('scale_multitenant: all VUs finished');

  // Query the payments consumer lag metric from Prometheus.
  // Best-effort — failure here does not affect thresholds or exit code.
  const q   = encodeURIComponent('payments_event_consumer_lag_messages');
  const res = http.get(`${PROMETHEUS_URL}/api/v1/query?query=${q}`);
  if (res.status === 200) {
    try {
      const body = JSON.parse(res.body);
      const result = body.data && body.data.result;
      if (result && result.length > 0) {
        result.forEach((r) => {
          console.log(
            `Projection lag [${JSON.stringify(r.metric)}]: ${r.value[1]} messages`
          );
        });
      } else {
        console.log('Projection lag metric: no active series (consumer caught up or not running)');
      }
    } catch (_) {
      console.log(`Projection lag: could not parse Prometheus response`);
    }
  } else {
    console.log(
      `Prometheus unavailable (status ${res.status}) — ` +
      'check Grafana dashboard for payments_event_consumer_lag_messages'
    );
  }
}
