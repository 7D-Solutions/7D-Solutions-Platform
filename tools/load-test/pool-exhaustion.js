/**
 * k6 load test — connection pool exhaustion probe.
 *
 * Targets /api/health which exercises the DB pool (SELECT 1) on every
 * request that reaches the service. The module's rate limiter will return
 * 429 for excess requests — those are expected and not counted as errors.
 * Only 503 responses indicate pool exhaustion.
 *
 * Run via: ./tools/load-test/run.sh --module <name> --concurrency 200 --duration 30s
 *
 * Pass criteria (asserted by k6 thresholds):
 *   - p99 response time < 2s  (across all responses including 429s)
 *   - Zero HTTP 503 responses (pool exhaustion)
 *   - "True error" rate < 1%  (5xx responses other than 503, or timeouts)
 */

import http from 'k6/http';
import { check } from 'k6';
import { Rate } from 'k6/metrics';

// 503 = pool exhaustion — the critical failure mode we're testing for
const poolExhaustion503 = new Rate('pool_exhaustion_503');
// True errors: 5xx (excluding 503) or network failures
const trueErrors = new Rate('true_errors');

const MODULE_HOST = __ENV.MODULE_HOST || 'localhost';
const MODULE_PORT = __ENV.MODULE_PORT || '8080';
const BASE_URL = `http://${MODULE_HOST}:${MODULE_PORT}`;

export const options = {
  vus: parseInt(__ENV.VUS || '200'),
  duration: __ENV.DURATION || '30s',
  thresholds: {
    // p99 latency must stay under 2 seconds (includes fast 429 responses)
    http_req_duration: ['p(99)<2000'],
    // Zero pool exhaustion 503s allowed — the primary assertion
    pool_exhaustion_503: ['rate==0'],
    // True errors (non-429 failures) under 1%
    true_errors: ['rate<0.01'],
  },
};

export default function () {
  // /api/health runs SELECT 1 against the pool for requests that get through
  // the rate limiter. Under concurrency pressure, expect 429s from rate limiting
  // — those are not pool errors.
  const res = http.get(`${BASE_URL}/api/health`, {
    headers: { 'Accept': 'application/json' },
    timeout: '3s',
  });

  check(res, {
    'not 503 pool exhaustion': (r) => r.status !== 503,
    'response under 2s': (r) => r.timings.duration < 2000,
    // 200 = healthy, 429 = rate limited (expected), both are acceptable
    'acceptable response (200 or 429)': (r) => r.status === 200 || r.status === 429,
  });

  poolExhaustion503.add(res.status === 503);

  // True error: any 5xx (except 503 which is pool-specific) or connection failure
  const is5xx = res.status >= 500 && res.status <= 599;
  const isPoolError = res.status === 503;
  trueErrors.add((is5xx && !isPoolError) || res.status === 0);
}

export function handleSummary(data) {
  const p99 = data.metrics.http_req_duration?.values?.['p(99)'];
  const trueErrRate = data.metrics.true_errors?.values?.rate || 0;
  const exhaustionRate = data.metrics.pool_exhaustion_503?.values?.rate || 0;

  const passed =
    (p99 === undefined || p99 < 2000) &&
    exhaustionRate === 0 &&
    trueErrRate < 0.01;

  const summary = {
    module: __ENV.MODULE_NAME || 'unknown',
    passed,
    p99_ms: p99 !== undefined ? Math.round(p99) : null,
    true_error_rate: trueErrRate,
    pool_exhaustion_503_rate: exhaustionRate,
    vus: parseInt(__ENV.VUS || '200'),
    duration: __ENV.DURATION || '30s',
  };

  console.log('\n--- POOL LOAD TEST SUMMARY ---');
  console.log(JSON.stringify(summary, null, 2));

  if (!passed) {
    console.log('\nFAIL: One or more thresholds breached.');
    if (p99 !== undefined && p99 >= 2000) {
      console.log(`  p99 latency ${Math.round(p99)}ms exceeds 2000ms threshold`);
    }
    if (exhaustionRate > 0) {
      console.log(`  Pool exhaustion 503s detected (rate: ${(exhaustionRate * 100).toFixed(2)}%)`);
    }
    if (trueErrRate >= 0.01) {
      console.log(`  True error rate ${(trueErrRate * 100).toFixed(2)}% exceeds 1% threshold`);
    }
  } else {
    console.log('\nPASS: All thresholds met (pool not exhausted).');
  }

  return {
    'stdout': JSON.stringify(summary, null, 2),
  };
}
